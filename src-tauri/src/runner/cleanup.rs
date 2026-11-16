//! Automatic removal of outdated runner images (saves disk space on the
//! students' machines).
//!
//! Runs in the background after the images of the current version were
//! prepared (app start) or updated ("Images aktualisieren"), in the engine
//! the app is connected to (Docker or Podman, same API). Per own repository
//! (`<IMAGE_REGISTRY>/code-pruefer-<sprache>`) it keeps the current version
//! and the highest release version below it ("Vorgängerversion") and
//! removes older tags, see [`images_to_remove`]. Errors are only logged.
//!
//! Removal goes through `DELETE /images/<ref>` without `force`: a reference
//! of an image with further tags is only untagged, an image used by a
//! container is left alone (the engine refuses). Dangling layers are not
//! pruned (`/images/prune` cannot be limited to our images).

use std::collections::HashSet;

use bollard::query_parameters::{ListImagesOptionsBuilder, RemoveImageOptionsBuilder};
use bollard::Docker;
use semver::Version;

use super::docker::split_ref;
use super::languages;
use super::RunnerError;

/// One tag in the engine's image store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    /// `<repo>:<tag>`.
    pub reference: String,
    /// Image ID the tag points to (`sha256:…`, may be empty).
    pub id: String,
}

impl ImageRef {
    pub fn new(reference: &str, id: &str) -> Self {
        Self {
            reference: reference.to_string(),
            id: id.to_string(),
        }
    }
}

/// What a tag of an own repository says about its version.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TagKind {
    /// `x.y.z`.
    Release(Version),
    /// `x.y.z-<pre>` (e.g. `-qas.3`).
    PreRelease(Version),
    /// `sha-<commit>`: version only known via the image ID.
    Sha,
    /// `dev`, `edge`, `latest`, build metadata, anything unparsable: kept.
    Other,
}

fn classify(tag: &str) -> TagKind {
    if tag.starts_with("sha-") {
        return TagKind::Sha;
    }
    match Version::parse(tag) {
        Ok(v) if !v.build.is_empty() => TagKind::Other,
        Ok(v) if v.pre.is_empty() => TagKind::Release(v),
        Ok(v) => TagKind::PreRelease(v),
        Err(_) => TagKind::Other,
    }
}

/// `x.y.z` without pre-release.
fn core(v: &Version) -> Version {
    Version::new(v.major, v.minor, v.patch)
}

/// The references to remove. Pure, unit-tested.
///
/// Per repository in `own_repos` (others are never touched), and only if
/// the tag `current` itself is present there:
/// - kept: `current`, the previous version = the highest present release
///   (`x.y.z`) below `current`, every higher version (newer app installed in
///   parallel / downgrade), `dev`, `edge`, `latest` and unparsable tags;
/// - removed: releases below the previous version;
/// - pre-releases (`x.y.z-qas.N`): removed if their `x.y.z` is below the
///   previous version, i.e. the QAS builds of the current and the previous
///   version are kept (they may be the image an installed test build uses);
/// - `sha-…` tags: removed only if their image ID is that of a removed tag
///   and of no kept versioned tag of the same repository (otherwise kept);
/// - nothing in `protect` (developer overrides) is ever removed.
///
/// Without a previous version nothing is removed.
pub fn images_to_remove(
    current: &Version,
    own_repos: &[&str],
    protect: &[String],
    images: &[ImageRef],
) -> Vec<String> {
    let mut remove: Vec<String> = Vec::new();
    for repo in own_repos {
        let tags: Vec<(&ImageRef, TagKind)> = images
            .iter()
            .filter(|i| split_ref(&i.reference).0 == *repo)
            .map(|i| (i, classify(split_ref(&i.reference).1)))
            .collect();
        let current_present = tags.iter().any(
            |(_, k)| matches!(k, TagKind::Release(v) | TagKind::PreRelease(v) if v == current),
        );
        if !current_present {
            continue;
        }
        let Some(previous) = tags
            .iter()
            .filter_map(|(_, k)| match k {
                TagKind::Release(v) if v < current => Some(v),
                _ => None,
            })
            .max()
            .cloned()
        else {
            continue;
        };
        let previous_core = core(&previous);
        let outdated = |k: &TagKind| match k {
            TagKind::Release(v) => *v < previous,
            TagKind::PreRelease(v) => core(v) < previous_core,
            TagKind::Sha | TagKind::Other => false,
        };
        let protected = |i: &ImageRef| protect.contains(&i.reference);
        let mut removed_ids: HashSet<&str> = HashSet::new();
        let mut kept_ids: HashSet<&str> = HashSet::new();
        for (image, kind) in &tags {
            if matches!(kind, TagKind::Sha) {
                continue;
            }
            if outdated(kind) && !protected(image) {
                remove.push(image.reference.clone());
                removed_ids.insert(image.id.as_str());
            } else {
                kept_ids.insert(image.id.as_str());
            }
        }
        for (image, kind) in &tags {
            let id = image.id.as_str();
            if matches!(kind, TagKind::Sha)
                && !id.is_empty()
                && removed_ids.contains(id)
                && !kept_ids.contains(id)
                && !protected(image)
            {
                remove.push(image.reference.clone());
            }
        }
    }
    remove.sort();
    remove.dedup();
    remove
}

/// Every tag in the engine's image store (one entry per tag).
pub async fn list_refs(docker: &Docker) -> Result<Vec<ImageRef>, RunnerError> {
    let opts = ListImagesOptionsBuilder::default().all(false).build();
    Ok(docker
        .list_images(Some(opts))
        .await?
        .into_iter()
        .flat_map(|summary| {
            let id = summary.id;
            summary
                .repo_tags
                .into_iter()
                .filter(|t| !t.is_empty() && !t.contains("<none>"))
                .map(move |reference| ImageRef {
                    reference,
                    id: id.clone(),
                })
        })
        .collect())
}

/// Result of [`remove_outdated_in`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupReport {
    pub removed: Vec<String>,
    /// `(reference, reason)` of removals the engine refused (e.g. image in
    /// use by a container).
    pub failed: Vec<(String, String)>,
}

/// Removes the outdated tags of `own_repos` (see [`images_to_remove`])
/// without force.
pub async fn remove_outdated_in(
    docker: &Docker,
    current: &Version,
    own_repos: &[&str],
    protect: &[String],
) -> Result<CleanupReport, RunnerError> {
    let refs = list_refs(docker).await?;
    let mut report = CleanupReport::default();
    for reference in images_to_remove(current, own_repos, protect, &refs) {
        let opts = RemoveImageOptionsBuilder::default()
            .force(false)
            .noprune(false)
            .build();
        match docker.remove_image(&reference, Some(opts), None).await {
            Ok(_) => report.removed.push(reference),
            Err(e) => report
                .failed
                .push((reference, super::docker::describe_error(&e))),
        }
    }
    Ok(report)
}

/// The cleanup for the app's languages and version; never touches the
/// developer overrides (`CODE_PRUEFER_<LANG>_IMAGE`, debug builds only).
/// Errors are logged, never returned.
pub async fn remove_outdated(docker: &Docker) {
    let Ok(current) = Version::parse(languages::IMAGE_TAG) else {
        eprintln!(
            "images: Aufräumen übersprungen, Version {} nicht lesbar",
            languages::IMAGE_TAG
        );
        return;
    };
    let repos: Vec<&str> = languages::LANGUAGES
        .iter()
        .map(|l| split_ref(l.default_image).0)
        .collect();
    let protect: Vec<String> = languages::LANGUAGES
        .iter()
        .filter_map(|l| l.image_override())
        .collect();
    match remove_outdated_in(docker, &current, &repos, &protect).await {
        Ok(report) => {
            for reference in &report.removed {
                eprintln!("images: veraltetes Image entfernt: {reference}");
            }
            for (reference, reason) in &report.failed {
                eprintln!("images: veraltetes Image {reference} nicht entfernt: {reason}");
            }
        }
        Err(e) => eprintln!("images: Aufräumen fehlgeschlagen: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PY: &str = "ghcr.io/bernhardgoetz/code-pruefer-python";
    const JAVA: &str = "ghcr.io/bernhardgoetz/code-pruefer-java";
    const REPOS: &[&str] = &[PY, JAVA];

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    /// One image per reference, IDs derived from the reference unless given.
    fn refs(list: &[&str]) -> Vec<ImageRef> {
        list.iter()
            .map(|r| ImageRef::new(r, &format!("sha256:{r}")))
            .collect()
    }

    fn run(current: &str, images: &[ImageRef]) -> Vec<String> {
        images_to_remove(&v(current), REPOS, &[], images)
    }

    fn py(tag: &str) -> String {
        format!("{PY}:{tag}")
    }

    #[test]
    fn only_current() {
        assert!(run("1.2.0", &refs(&[&py("1.2.0")])).is_empty());
    }

    #[test]
    fn keeps_current_and_previous_removes_older() {
        let images = refs(&[
            &py("1.2.0"),
            &py("1.1.3"),
            &py("1.1.2"),
            &py("1.0.0"),
            &py("0.9.0"),
        ]);
        assert_eq!(
            run("1.2.0", &images),
            vec![py("0.9.0"), py("1.0.0"), py("1.1.2")]
        );
    }

    #[test]
    fn previous_is_highest_present_not_current_minus_one() {
        // 1.1.x missing: 1.0.5 is the previous version.
        let images = refs(&[&py("1.2.0"), &py("1.0.5"), &py("1.0.4")]);
        assert_eq!(run("1.2.0", &images), vec![py("1.0.4")]);
    }

    #[test]
    fn nothing_without_current_tag() {
        // Current image not (yet) present: keep everything.
        let images = refs(&[&py("1.1.0"), &py("1.0.0")]);
        assert!(run("1.2.0", &images).is_empty());
    }

    #[test]
    fn qas_tags() {
        let images = refs(&[
            &py("1.2.0"),
            &py("1.2.0-qas.7"),
            &py("1.1.0"),
            &py("1.1.0-qas.3"),
            &py("1.0.0"),
            &py("1.0.0-qas.1"),
            &py("0.9.0-qas.2"),
        ]);
        assert_eq!(
            run("1.2.0", &images),
            vec![py("0.9.0-qas.2"), py("1.0.0"), py("1.0.0-qas.1")]
        );
    }

    #[test]
    fn qas_of_newer_version_and_no_previous_release() {
        // Only pre-releases below current: no previous release, keep all.
        let images = refs(&[&py("1.2.0"), &py("1.1.0-qas.1"), &py("1.3.0-qas.1")]);
        assert!(run("1.2.0", &images).is_empty());
    }

    #[test]
    fn higher_versions_kept() {
        let images = refs(&[
            &py("1.2.0"),
            &py("2.0.0"),
            &py("1.10.0"),
            &py("1.1.0"),
            &py("1.0.0"),
        ]);
        assert_eq!(run("1.2.0", &images), vec![py("1.0.0")]);
        // Semver order, not string order: 1.10.0 > 1.9.0.
        let images = refs(&[&py("1.10.0"), &py("1.9.0"), &py("1.2.0")]);
        assert_eq!(run("1.10.0", &images), vec![py("1.2.0")]);
    }

    #[test]
    fn dev_and_foreign_never_removed() {
        let images = refs(&[
            &py("1.2.0"),
            &py("1.1.0"),
            &py("dev"),
            &py("edge"),
            &py("latest"),
            "code-pruefer-python:dev",
            "localhost/code-pruefer-python:1.0.0",
            "ghcr.io/someoneelse/code-pruefer-python:1.0.0",
            "ghcr.io/bernhardgoetz/other:1.0.0",
            "python:3.12",
            "docker.io/library/eclipse-temurin:21",
        ]);
        assert!(run("1.2.0", &images).is_empty());
    }

    #[test]
    fn broken_tags_kept() {
        let images = refs(&[
            &py("1.2.0"),
            &py("1.1.0"),
            &py("1.0"),
            &py("v1.0.0"),
            &py("01.0.0"),
            &py("1.0.0+build.5"),
            &py("1.0.0.0"),
            &py(""),
        ]);
        assert!(run("1.2.0", &images).is_empty());
    }

    #[test]
    fn several_languages_independent() {
        let images = refs(&[
            &py("1.2.0"),
            &py("1.1.0"),
            &py("1.0.0"),
            // Java: current missing -> untouched.
            &format!("{JAVA}:1.1.0"),
            &format!("{JAVA}:1.0.0"),
            &format!("{JAVA}:0.9.0"),
        ]);
        assert_eq!(run("1.2.0", &images), vec![py("1.0.0")]);
        let images = refs(&[
            &py("1.2.0"),
            &py("1.0.0"),
            &py("0.9.0"),
            &format!("{JAVA}:1.2.0"),
            &format!("{JAVA}:1.1.0"),
            &format!("{JAVA}:1.0.0"),
        ]);
        // Previous version per language: python 1.0.0, java 1.1.0.
        assert_eq!(
            run("1.2.0", &images),
            vec![format!("{JAVA}:1.0.0"), py("0.9.0")]
        );
    }

    #[test]
    fn current_with_pre_release() {
        // App version 1.2.0-qas.3: 1.2.0 is higher (kept), 1.1.0 previous.
        let images = refs(&[
            &py("1.2.0-qas.3"),
            &py("1.2.0"),
            &py("1.2.0-qas.2"),
            &py("1.1.0"),
            &py("1.1.0-qas.1"),
            &py("1.0.0"),
        ]);
        assert_eq!(run("1.2.0-qas.3", &images), vec![py("1.0.0")]);
    }

    #[test]
    fn sha_tags_only_with_unambiguous_removed_image() {
        let mut images = vec![
            ImageRef::new(&py("1.2.0"), "sha256:c"),
            ImageRef::new(&py("sha-ccc"), "sha256:c"),
            ImageRef::new(&py("1.1.0"), "sha256:b"),
            ImageRef::new(&py("sha-bbb"), "sha256:b"),
            ImageRef::new(&py("1.0.0"), "sha256:a"),
            ImageRef::new(&py("1.0.0-qas.4"), "sha256:a"),
            ImageRef::new(&py("sha-aaa"), "sha256:a"),
            // Unknown image: keep.
            ImageRef::new(&py("sha-xxx"), "sha256:x"),
            ImageRef::new(&py("sha-empty"), ""),
        ];
        assert_eq!(
            run("1.2.0", &images),
            vec![py("1.0.0"), py("1.0.0-qas.4"), py("sha-aaa")]
        );
        // Old release shares its image with the kept previous version
        // (unchanged image): the old tag goes, the sha tag stays.
        images.push(ImageRef::new(&py("0.9.0"), "sha256:b"));
        images.push(ImageRef::new(&py("sha-old"), "sha256:b"));
        let removed = run("1.2.0", &images);
        assert!(removed.contains(&py("0.9.0")));
        assert!(!removed.contains(&py("sha-old")));
        assert!(!removed.contains(&py("sha-bbb")));
    }

    #[test]
    fn protected_overrides_never_removed() {
        let images = refs(&[&py("1.2.0"), &py("1.1.0"), &py("1.0.0"), &py("0.9.0")]);
        let removed = images_to_remove(&v("1.2.0"), REPOS, &[py("1.0.0")], &images);
        assert_eq!(removed, vec![py("0.9.0")]);
    }

    #[test]
    fn registry_with_port_and_unknown_repo_list() {
        let repo = "localhost:5000/code-pruefer-test-cleanup";
        let images = refs(&[
            &format!("{repo}:1.2.0"),
            &format!("{repo}:1.1.0"),
            &format!("{repo}:1.0.0"),
        ]);
        assert_eq!(
            images_to_remove(&v("1.2.0"), &[repo], &[], &images),
            vec![format!("{repo}:1.0.0")]
        );
        assert!(images_to_remove(&v("1.2.0"), &[], &[], &images).is_empty());
    }

    #[test]
    fn app_repos_are_own_registry() {
        for lang in languages::LANGUAGES {
            let repo = split_ref(lang.default_image).0;
            assert_eq!(
                repo,
                format!("{}/code-pruefer-{}", languages::IMAGE_REGISTRY, lang.id)
            );
        }
        assert!(Version::parse(languages::IMAGE_TAG).is_ok());
    }
}
