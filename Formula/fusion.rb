class Fusion < Formula
  desc "Fast, lightweight, cross-platform AI coding assistant"
  homepage "https://fusioncode.app"
  url "https://github.com/theaungmyatmoe/fusion/archive/refs/tags/v2.0.0-alpha.1.tar.gz"
  sha256 "76a1a5f91f4f75109c7d5d3e5dab58462c7ff0cc53d9e3dfd7ab635d48600e46"
  license "MIT"
  head "https://github.com/theaungmyatmoe/fusion.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args

    generate_completions_from_executable(bin/"fusion", "--generate-completion")
  end

  test do
    assert_match "fusion #{version}", shell_output("#{bin}/fusion --version")
  end
end
