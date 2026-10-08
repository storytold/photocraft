# Homebrew cask for PhotoCraft, published in the storytold/homebrew-tap tap:
#   brew install --cask storytold/tap/photocraft
# Source: https://github.com/storytold/photocraft/tree/main/packaging/homebrew
# (packaging/homebrew/update.sh sets version and sha256 for each release.)
cask "photocraft" do
  version "0.5.0"
  sha256 "dff8c8105d5938d46fa4ba29559d3efc0f1ea195a5392d36cf62bc14ea2de5e7"

  url "https://github.com/storytold/photocraft/releases/download/v#{version}/photocraft-#{version}-macos-universal.dmg"
  name "PhotoCraft"
  desc "Image editor with layers, masks, type and PSD files"
  homepage "https://getartcraft.com/apps/photocraft"

  livecheck do
    url :url
    strategy :github_latest
  end

  # LSMinimumSystemVersion in packaging/macos/Info.plist.in.
  depends_on macos: :big_sur

  app "PhotoCraft.app"

  uninstall quit: "ai.storyteller.photocraft"

  # Preferences, presets, recovery autosaves and window layout (apps/photocraft/src/app_dirs.rs),
  # plus what macOS keeps per bundle id.
  zap trash: [
    "~/Library/Application Support/Photocraft",
    "~/Library/Preferences/ai.storyteller.photocraft.plist",
    "~/Library/Saved Application State/ai.storyteller.photocraft.savedState",
  ]
end
