# upm

A Unity package registry for KSP Retooled. This repo holds the generator and
package configs. GitHub Pages serves the generated metadata, and each project's
GitHub releases host its package tarballs.

## Add a package

The package's GitHub releases need a tarball with `package/package.json` at its
root, as `npm pack` produces. Add a config for it to `configs/`. See
[com.example.mypackage.toml](examples/com.example.mypackage.toml) for the options.

Merging the config to `main` publishes its existing releases. To preview the
generated files locally in the ignored `registry/` directory, run

```sh
cargo run -- generate com.example.mypackage --all
```

Published versions don't change. If a release asset is replaced after that, its
hashes stop matching and Unity will refuse it, so publish a new version instead.

## Publish a release

Once the package has a config here, add this workflow to the package repo as
`.github/workflows/publish-upm.yml`. It starts the registry update with a token
from the organization's automation app, so the package repo needs the
organization secrets `APP_CLIENT_ID` and `APP_PRIVATE_KEY`. The app needs
Actions write access to `KSP1Retooled/upm`.

```yaml
name: Publish to UPM
on:
  release:
    types: [published]

jobs:
  publish:
    permissions:
      contents: write
    uses: KSP1Retooled/upm/.github/workflows/publish.yml@main
    with:
      package-path: .  # Directory containing the Unity package.json
    secrets:
      APP_CLIENT_ID: ${{ secrets.APP_CLIENT_ID }}
      APP_PRIVATE_KEY: ${{ secrets.APP_PRIVATE_KEY }}
```

The workflow packs the tagged package, attaches the `.tgz` to the release, and
waits for the registry update and deployment. Reruns reuse the existing asset.
If your release job already uploads the tarball, omit `package-path` and use
`contents: read`. You can call the same workflow after that job with `needs`
and pass its release tag via `with.tag`.

To publish an existing release manually:

```sh
gh workflow run update.yml -R KSP1Retooled/upm -f repo=<owner/name> -f tag=<tag>
```

Everything else about the version comes from the tarball's `package.json`.

## Commands

| Command | |
| --- | --- |
| `upm-registry generate [packages]` | Add the latest release of each package |
| `upm-registry generate --tag <tag>` | Add the release with that tag |
| `upm-registry generate --all` | Add every release |
| `upm-registry generate --repo <owner/name>` | Limit to packages from that repo |
| `upm-registry index` | Rebuild the search index and host files |
| `upm-registry check` | Validate the configs and the registry |

Set `GITHUB_TOKEN` to avoid GitHub's anonymous rate limit.

## Hosting

Use `https://ksp1retooled.github.io/upm/` as the scoped registry URL in Unity.
Each update starts from `upm/` on the `gh-pages` branch of
`KSP1Retooled/ksp1retooled.github.io`, preserving previously published versions
and their checksums. Actions queues updates, validates the generated files,
keeps a downloadable `registry` artifact for seven days, and deploys the same
files back to `upm/`.

Package release workflows update the requested tag. Changes to the generator,
configs, or deployment workflow on `main` sync all configured release histories.
To run that sync manually, use

```sh
gh workflow run deploy.yml -R KSP1Retooled/upm
```

The workflow uses the same GitHub App as the site publisher. Make the Actions
secrets `APP_CLIENT_ID` and `APP_PRIVATE_KEY` available to this repo.
The app needs Contents write access to `KSP1Retooled/ksp1retooled.github.io`.
