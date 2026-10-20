# Storage Navigator Documentation

The Storage Navigator app's service-specific documentation. The pages are Markdown sources under [`docs/`](docs/) and read as-is.

NVIDIA builds the rendered documentation site from these sources internally, with `repo_docs` tooling (the `repo.sh`/`repo.bat` launchers, `repo.toml`, the packman bootstrap and the theme assets) that is kept in NVIDIA's source repository. There, `./repo.sh docs` (or `repo.bat docs` on Windows) builds the site into `_build/`, and `./build_pdf.sh` produces a PDF.
