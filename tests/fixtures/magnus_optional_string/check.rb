# frozen_string_literal: true

require ARGV.fetch(0)

raise "missing argument changed" unless optional_string_fixture.nil?
raise "nil changed" unless optional_string_fixture(nil).nil?
raise "string changed" unless optional_string_fixture("value") == "value"
raise "empty string changed" unless optional_string_fixture("") == ""

count = 0
[5, true, [], {}].each do |value|
  begin
    optional_string_fixture(value)
  rescue TypeError
    count += 1
    next
  end

  raise "expected TypeError for #{value.class}"
end

raise "wrong case count" unless count == 4

puts("PASS: 4 valid controls, 4 TypeError cases")
