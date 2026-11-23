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
are signed before they reach the registry. This repo has these pieces for a
package repo's release workflow:

- The `check-release` action checks that the tag is `v` plus the version in
  `package.json`. If you give it the paths of committed binaries and of their
  source, it also checks that the binaries are newer than the source.
- The `pack` action runs `npm pack` and fails if the tarball is missing a file
  from `expect`, or holds a file that matches a pattern in `forbid`.
- The `sign` action signs the tarball with Unity's upm CLI. It runs on Linux
  x64.
- The `publish.yml` workflow adds the release to the registry and waits for
  the deployment.

Once the package has a config here, add this workflow to the package repo as
`.github/workflows/publish-upm.yml`. Everything runs on GitHub-hosted runners.

```yaml
name: Publish to UPM
on:
  release:
    types: [published]

env:
  TAG: ${{ github.event.release.tag_name }}

jobs:
  pack:
    runs-on: ubuntu-latest
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@v5
        with:
          ref: refs/tags/${{ env.TAG }}
          fetch-depth: 0
          persist-credentials: false
      - uses: KSP1Retooled/upm/check-release@main
        with:
          tag: ${{ env.TAG }}
          source: |
            Native~/CMakeLists.txt
            Native~/src
          binaries: Plugins/x86_64/MyPlugin.dll
      # Fetch or build anything else the package needs here.
      - id: pack
        uses: KSP1Retooled/upm/pack@main
        with:
          expect: |
            Plugins/x86_64/MyPlugin.dll
            Plugins/x86_64/MyPlugin.dll.meta
      - id: sign
        uses: KSP1Retooled/upm/sign@main
        with:
          tarball: ${{ steps.pack.outputs.path }}
          organization-id: ${{ vars.UPM_ORG_ID }}
          key-id: ${{ secrets.UPM_SERVICE_ACCOUNT_KEY_ID }}
          key: ${{ secrets.UPM_SERVICE_ACCOUNT_KEY }}
      - env:
          GH_TOKEN: ${{ github.token }}
          SIGNED: ${{ steps.sign.outputs.path }}
        run: gh release upload --repo "$GITHUB_REPOSITORY" -- "$TAG" "$SIGNED"

  publish:
    needs: pack
    permissions:
      contents: read
    uses: KSP1Retooled/upm/.github/workflows/publish.yml@main
    with:
      tag: ${{ github.event.release.tag_name }}
    secrets:
      APP_CLIENT_ID: ${{ secrets.APP_CLIENT_ID }}
      APP_PRIVATE_KEY: ${{ secrets.APP_PRIVATE_KEY }}
```

Leave out `source` and `binaries` if the package has no native plug-in.

The package repo needs access to the organization variable `UPM_ORG_ID` (the
Unity organization ID) and these organization secrets:

- `UPM_SERVICE_ACCOUNT_KEY_ID` and `UPM_SERVICE_ACCOUNT_KEY`: the key
  of a Unity service account with the **Package Manager Package Signer** role.
- `APP_CLIENT_ID` and `APP_PRIVATE_KEY`: the organization's automation app,
  which needs Actions write access to `KSP1Retooled/upm`.

To publish an existing release manually:

```sh
gh workflow run update.yml -R KSP1Retooled/upm -f repo=<owner/name> -f tag=<tag>
```

Everything else about the version comes from the tarball's `package.json`.

## Commit native binaries

Packages with a native plug-in commit its binaries, so that a release ships
exactly what was built from the tagged source. `commit-binaries.yml` commits
the binaries that the earlier jobs of a run uploaded as artifacts to the branch
that started the run. Build the plug-in in your own jobs, then add

```yaml
  commit:
    needs: build
    if: github.ref_type == 'branch'
    permissions:
      contents: write
    uses: KSP1Retooled/upm/.github/workflows/commit-binaries.yml@main
    with:
      artifacts: native-*
      path: Plugins/x86_64
```

The artifacts' files are merged into `path`. A push made with the run's token
doesn't start another run, so the build doesn't loop.

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
