require "formula"
require "tmpdir"
require "open3"

path = Pathname.new(ARGV.fetch(0))
archive = File.expand_path(ARGV.fetch(1))
klass = Formulary.load_formula_from_path("webskiff", path, flags: [], ignore_errors: false)
formula = klass.new("webskiff", path, :stable)

Dir.mktmpdir("webskiff-brew-test-") do |directory|
  raise "unpack failed" unless system("tar", "-xzf", archive, "-C", directory)

  bundle = Dir.glob(File.join(directory, "webskiff-*")).find { |item| File.directory?(item) }
  raise "missing package" unless bundle

  prefix = Pathname.new(directory)/"prefix"
  {
    prefix: prefix,
    libexec: prefix/"libexec",
    bin: prefix/"bin",
    pkgshare: prefix/"share/webskiff",
  }.each do |name, value|
    formula.define_singleton_method(name) { value }
  end
  Dir.chdir(bundle) { formula.install }
  raise "missing executable wrapper" unless (formula.bin/"webskiff").executable?

  home = File.join(directory, "home")
  Dir.mkdir(home)
  version, status = Open3.capture2e({ "HOME" => home }, (formula.bin/"webskiff").to_s, "--version")
  raise version unless status.success? && version.include?("webskiff #{formula.version}")

  output, status = Open3.capture2e({ "HOME" => home }, (formula.bin/"webskiff").to_s, "--selftest")
  raise output unless status.success? && output.include?("ALL PASSED")

  puts "Homebrew install method, exec wrapper, version and engine self-test passed in an isolated prefix."
end
