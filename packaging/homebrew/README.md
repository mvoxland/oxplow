# Homebrew Cask

`oxplow.rb` is the source of truth for the Homebrew Cask
published to the private tap at
[`nvoxland/homebrew-oxplow`](https://github.com/nvoxland/homebrew-oxplow).
The version + sha256 in this file are placeholders — the
release workflow rewrites them on every tag push and force-pushes
the result to the tap repo.

## What the cask does

Installs the unsigned Apple-Silicon DMG from the matching GitHub
Release. Homebrew strips the `com.apple.quarantine` attribute on
install by default, which is the whole reason the cask exists —
without it, double-clicking the DMG hits the "Oxplow.app is
damaged" Gatekeeper wall on recent macOS.

## Updating manually

Normally CI does this. If the publish job fails or you want to
push an out-of-band update:

```sh
ver="0.4.0"
url="https://github.com/nvoxland/oxplow/releases/download/v${ver}/Oxplow_${ver}_aarch64.dmg"
sha=$(curl -sSL "$url" | shasum -a 256 | awk '{print $1}')

# In a checkout of the tap repo:
sed -i '' \
  -e "s/^  version \".*\"/  version \"${ver}\"/" \
  -e "s/^  sha256 \".*\"/  sha256 \"${sha}\"/" \
  Casks/oxplow.rb
git add Casks/oxplow.rb
git commit -m "oxplow ${ver}"
git push origin main
```

## Bootstrapping the tap

The first time you set this up, create
`https://github.com/nvoxland/homebrew-oxplow` (it must be named
`homebrew-*` for `brew tap nvoxland/oxplow` to find it). Layout:

```
homebrew-oxplow/
  Casks/
    oxplow.rb        # mirrors this file, with concrete version + sha
  README.md          # one-liner: brew install --cask nvoxland/oxplow/oxplow
```

The release workflow needs a personal access token with
`repo` scope on the tap repo, exposed to the workflow as the
`HOMEBREW_TAP_TOKEN` secret (see `.github/workflows/release.yml`).
