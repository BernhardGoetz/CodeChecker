## Privacy policy
Code-Prüfer runs entirely on the user's computer. It has no user accounts, no telemetry,
no analytics and no crash reporting, and it does not send code, tasks, test cases or any
other user data to the author or to third parties.

This program will not transfer any information to other networked systems unless specifically requested by the user or the person installing or operating it. The following connections are the only exceptions or are started on request, and none of them transmits user code, tasks or test cases:

Started without a click:

- **Update check (installed versions only, can be turned off):** about five seconds
  after the start and then at most once a day (default; weekly or never in the
  settings), and when the user clicks "Jetzt nach Updates suchen", the app downloads
  `https://github.com/bernhardgoetz/codechecker/releases/latest/download/latest.json`
  over HTTPS, and on "Jetzt aktualisieren" the installer it names from the same GitHub
  release. No user data is sent (GitHub sees the IP address and a generic user agent);
  every update is verified against the project's update signing key before it is
  installed. Builds without that key and the portable Windows version never check.
  GitHub's privacy statement applies:
  <https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement>.
- **Runner images (container engine):** the user's container engine (Docker or Podman)
  pulls the runner images from the GitHub Container Registry `ghcr.io` when a check or a
  test generation is started and the image is neither installed nor contained in the
  installer, or when the user clicks "Images aktualisieren". At start-up the app only
  loads the images bundled with the installer and never downloads them. GitHub's privacy
  statement applies:
  <https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement>.
- **Podman virtual machine (only if the user chose Podman):** the app starts the chosen
  container engine on its own when it is not running. If Podman has no virtual machine
  yet, the app runs `podman machine init` once, which makes Podman download its virtual
  machine image from the Podman project's servers (several hundred MB). The app itself
  sends nothing; Podman's own behaviour and the policies of its image hosts apply.

Started only on request:

- **AI model download (optional):** when the user clicks the download button in the
  model manager, the selected model file is downloaded from `huggingface.co` (and its
  hosts `*.huggingface.co`, `hf.co`, `*.hf.co`). Only a regular HTTPS file download is
  performed; no user data is sent. Hugging Face's privacy policy applies:
  <https://huggingface.co/privacy>.
- **Container engine installation (Windows installer):** if the chosen engine is missing
  and the user agrees, the installer runs `winget install` for Docker Desktop or Podman.
  winget contacts the Microsoft winget source and downloads the installer from the vendor
  (Docker, or the Podman project on GitHub). Microsoft's, Docker's and GitHub's privacy
  statements apply.
- Links to the Docker and Podman websites and to the project's release page are opened in
  the user's browser only when the user clicks them.

Local only and outside the application:

- The app talks to the local container engine (socket or named pipe) and to the bundled
  AI runtime on `127.0.0.1` (started with `--offline`). User code is executed in
  containers **without network access**.
- Docker Desktop and Podman, once started, may contact their vendors according to their
  own settings (for example update checks); this is outside Code-Prüfer.
