Given /^I am in a clean git repository$/ do
  setup_safe_folder
  in_project_folder do
    system "git init"
    system "touch a"
    system "git add a"
    system "git commit -m 'Initial commit.'"
  end
end

When /^I execute ti "(.*)"$/ do |cmd|
  in_project_folder do
    capture_output "ti #{cmd}"
  end
end

Then /^the output of `(.*)` should contain "(.*)"$/ do |cmd, text|
  in_project_folder do
    capture_output cmd
    output_of(cmd).should include(text)
  end
end

Then /^the output of `(.*)` should contain \/(.*)\/$/ do |cmd, regex|
  in_project_folder do
    capture_output cmd
    output_of(cmd).should match(/#{regex}/)
  end
end
