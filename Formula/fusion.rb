class Fusion < Formula
  desc "Fast, lightweight, cross-platform AI coding assistant"
  homepage "https://fusioncode.app"
  url "https://github.com/theaungmyatmoe/fusion/archive/refs/tags/v0.3.0.tar.gz"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license any_of: ["MIT", "Apache-2.0"]
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
