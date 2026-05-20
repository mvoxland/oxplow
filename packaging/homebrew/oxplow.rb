cask "oxplow" do
  version "0.0.0"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"

  url "https://github.com/nvoxland/oxplow/releases/download/v#{version}/Oxplow_#{version}_aarch64.dmg"
  name "Oxplow"
  desc "Project shell for supervising AI coding agents"
  homepage "https://github.com/nvoxland/oxplow"

  # CI only produces an Apple Silicon DMG today (`macos-latest`
  # runners are arm64). If Intel coverage is added later this cask
  # can grow a sha256 stanza with `:arm64`/`:x86_64` variants and
  # drop the `depends_on arch` line.
  depends_on macos: ">= :sonoma"
  depends_on arch: :arm64

  app "Oxplow.app"
  binary "#{appdir}/Oxplow.app/Contents/MacOS/oxplow"

  # Homebrew strips the com.apple.quarantine attribute on every
  # cask install by default (`brew install --cask` without
  # `--quarantine`), which is exactly the workaround we'd otherwise
  # tell users to run by hand. That's the whole point of this cask:
  # the bundle is unsigned + unnotarized, and without dropping the
  # quarantine attribute users hit the "Oxplow.app is damaged" wall.
  # No extra directive needed here — just don't pass `--quarantine`.

  zap trash: [
    "~/Library/Application Support/com.voxland.oxplow",
    "~/Library/Application Support/oxplow",
    "~/Library/Preferences/com.voxland.oxplow.plist",
    "~/Library/Saved Application State/com.voxland.oxplow.savedState",
    "~/Library/Caches/com.voxland.oxplow",
    "~/Library/WebKit/com.voxland.oxplow",
  ]
end
