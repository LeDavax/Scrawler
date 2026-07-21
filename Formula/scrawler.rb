class Scrawler < Formula
  desc "Portable semantic application runtime for agents"
  homepage "https://github.com/LeDavax/Scrawler"
  version "0.1.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/LeDavax/Scrawler/releases/download/v#{version}/scrawler-darwin-aarch64.tar.gz"
      sha256 "PLACEHOLDER_SHA256_DARWIN_ARM64"
    else
      url "https://github.com/LeDavax/Scrawler/releases/download/v#{version}/scrawler-darwin-x86_64.tar.gz"
      sha256 "PLACEHOLDER_SHA256_DARWIN_X86_64"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/LeDavax/Scrawler/releases/download/v#{version}/scrawler-linux-aarch64.tar.gz"
      sha256 "PLACEHOLDER_SHA256_LINUX_ARM64"
    else
      url "https://github.com/LeDavax/Scrawler/releases/download/v#{version}/scrawler-linux-x86_64.tar.gz"
      sha256 "PLACEHOLDER_SHA256_LINUX_X86_64"
    end
  end

  def install
    bin.install "scrawler"
  end

  test do
    assert_match "scrawler", shell_output("#{bin}/scrawler --help 2>&1", 2)
  end
end
