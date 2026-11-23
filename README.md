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

Package Manager warns about unsigned packages from Unity 6.3 on, so releases
are signed before they reach the registry:

- The `sign` action signs an `npm pack` tarball with Unity's upm CLI. It runs
  on Linux x64. The calling workflow attaches the signed tarball to the
  release.
- The `publish.yml` workflow adds the release to the registry and waits for
  the deployment.

Once the package has a config here, add this workflow to the package repo as
`.github/workflows/publish-upm.yml`. Build and pack the package in the `pack`
job, which can run on a GitHub-hosted runner. Only the `sign` job needs the
organization's runner.

```yaml
name: Publish to UPM
on:
  release:
    types: [published]

jobs:
  pack:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
        with:
          ref: refs/tags/${{ github.event.release.tag_name }}
          persist-credentials: false
      # Build anything the package needs here.
      - run: npm pack --ignore-scripts --pack-destination "$RUNNER_TEMP/package"
      - uses: actions/upload-artifact@v4
        with:
          name: package
          path: ${{ runner.temp }}/package/*.tgz

  sign:
    needs: pack
    runs-on: self-hosted
    container:
      image: ghcr.io/ksp1retooled/ksp-ci:1
    permissions:
      contents: write
    steps:
      - uses: actions/download-artifact@v4
        with:
          name: package
          path: ${{ runner.temp }}/package
      - id: tarball
        run: echo "path=$(ls "$RUNNER_TEMP"/package/*.tgz)" >> "$GITHUB_OUTPUT"
      - id: sign
        uses: KSP1Retooled/upm/sign@main
        with:
          tarball: ${{ steps.tarball.outputs.path }}
          organization-id: ${{ vars.UPM_ORG_ID }}
          key-id: ${{ secrets.UPM_SERVICE_ACCOUNT_KEY_ID }}
          key: ${{ secrets.UPM_SERVICE_ACCOUNT_KEY }}
      - env:
          GH_TOKEN: ${{ github.token }}
          TAG: ${{ github.event.release.tag_name }}
          SIGNED: ${{ steps.sign.outputs.path }}
        run: gh release upload --repo "$GITHUB_REPOSITORY" -- "$TAG" "$SIGNED"

  publish:
    needs: sign
    permissions:
      contents: read
    uses: KSP1Retooled/upm/.github/workflows/publish.yml@main
    with:
      tag: ${{ github.event.release.tag_name }}
    secrets:
      APP_CLIENT_ID: ${{ secrets.APP_CLIENT_ID }}
      APP_PRIVATE_KEY: ${{ secrets.APP_PRIVATE_KEY }}
```

The package repo needs access to the organization's runner, the organization
variable `UPM_ORG_ID` (the Unity organization ID), and these organization
secrets:

- `UPM_SERVICE_ACCOUNT_KEY_ID` and `UPM_SERVICE_ACCOUNT_KEY`: the key
  of a Unity service account with the **Package Manager Package Signer** role.
- `APP_CLIENT_ID` and `APP_PRIVATE_KEY`: the organization's automation app,
  which needs Actions write access to `KSP1Retooled/upm`.

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
