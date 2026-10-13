# Cask template: scripts/release.sh fills @VERSION@ and @SHA256@ and writes the result to
# target/release/bundle/tern.rb. Copy that into a tap (your-moon/homebrew-tern, Casks/tern.rb);
# then `brew install --cask your-moon/tern/tern`.
cask "tern" do
  version "@VERSION@"
  sha256 "@SHA256@"

  url "https://github.com/your-moon/tern/releases/download/v#{version}/tern-#{version}-macos-arm64.zip"
  name "tern"
  desc "Fast, low-memory SSH terminal"
  homepage "https://github.com/your-moon/tern"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on arch: :arm64
  depends_on macos: :big_sur

  app "tern.app"

  zap trash: [
    "~/Library/Application Support/tern",
    "~/Library/Logs/tern",
  ]
end
