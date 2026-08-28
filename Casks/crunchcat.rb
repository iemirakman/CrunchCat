cask "crunchcat" do
  version "1.0.0"
  sha256 "ae1667206dbef9cec56a8eaa154b133480899c63e59a005aa455a0d0bb62cceb"

  url "https://github.com/iemirakman/CrunchCat/releases/download/v#{version}/CrunchCat-v#{version}-macOS-AppleSilicon.dmg"
  name "CrunchCat"
  desc "Native macOS desktop application utilizing a Rust backend for high-performance file compression"
  homepage "https://github.com/iemirakman/CrunchCat"

  app "CrunchCat.app"
end
