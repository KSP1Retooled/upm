# upm

A Unity package registry for KSP Retooled. Package metadata lives in this repo and
is served as static files. The package tarballs stay on each project's GitHub
releases.

`upm-registry` turns the TOML files in `configs/` into the JSON in `registry/`.
Don't edit `registry/` by hand.

## Add a package

The package's GitHub releases need a tarball with `package/package.json` at its
root, as `npm pack` produces. Add a config for it to `configs/`. See
[com.example.mypackage.toml](examples/com.example.mypackage.toml) for the options.

Then publish its existing releases:

```sh
cargo run -- generate com.example.mypackage --all
```

Published versions don't change. If a release asset is replaced after that, its
hashes stop matching and Unity will refuse it, so publish a new version instead.

## Publish a release

From a workflow in the package's repo, with a token that can run workflows here:

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

Deploy `registry/` as a static site. On Cloudflare Pages, use no build command and
`registry` as the output directory. GitHub Pages works too.
