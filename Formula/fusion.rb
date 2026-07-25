class Fusion < Formula
  desc "Terminal-first AI coding agent made by Fusion AI"
  homepage "https://fusioncode.app"
  version "0.1.0"
  license "Apache-2.0"

  if OS.mac? && Hardware::CPU.arm?
    url "https://github.com/theaungmyatmoe/fusion/releases/download/v#{version}/fusion-v#{version}-aarch64-apple-darwin.tar.gz"
  elsif OS.mac? && Hardware::CPU.intel?
    url "https://github.com/theaungmyatmoe/fusion/releases/download/v#{version}/fusion-v#{version}-x86_64-apple-darwin.tar.gz"
  elsif OS.linux?
    url "https://github.com/theaungmyatmoe/fusion/releases/download/v#{version}/fusion-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
  end

  def install
    bin.install "fusion"
  end

  test do
    assert_match "fusion", shell_output("#{bin}/fusion --version")
  end
end
