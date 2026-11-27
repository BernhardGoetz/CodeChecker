# Runner-Image für Python-Abgaben des Code-Prüfers.
# Der Container wird von der App mit Netzwerk aus, read-only Root-FS und
# tmpfs unter /work und /tmp gestartet.
# Basis-Image per Digest (Manifest-Liste, multi-arch) gepinnt; der Tag dient
# nur der Lesbarkeit. Aktualisierung: Dependabot (docker) bzw.
# `docker buildx imagetools inspect python:<tag>`.
FROM python:3.15.2-slim-bookworm@sha256:96599df822d0e1c6bb700014a05c7f3c8cabc6e0f7ff3c4f9f69cee033b4b5d9

ENV LANG=C.UTF-8 \
    LC_ALL=C.UTF-8 \
    PYTHONUNBUFFERED=1 \
    PYTHONIOENCODING=utf-8

RUN groupadd --gid 1000 runner \
 && useradd --uid 1000 --gid 1000 --no-create-home --home-dir /tmp --shell /usr/sbin/nologin runner \
 && mkdir -p /work && chown 1000:1000 /work

USER 1000:1000
WORKDIR /work
CMD ["sleep", "infinity"]
