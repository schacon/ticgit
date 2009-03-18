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

When /^refresh my ti list index$/ do
  # running `ti list` generates the index that lets us refer to tickets by 
  # simple numbers (e.g., 1, 2, 3...) instead of by their tic ids (fee1e0, 851749, ...)
  in_project_folder do
    system "ti list > /dev/null"
  end
end

Then /^the output of `(.*)` (should|should not) contain "(.*)"$/ do |cmd, should, text|
  in_project_folder do
    if !File.exist?(output_file_for(cmd))
      capture_output cmd
    end
    if (should == "should")
      output_of(cmd).should include(text)
    else
      output_of(cmd).should_not include(text)
    end
  end
end

Then /^the output of `(.*)` (should|should not) contain \/(.*)\/$/ do |cmd, should, regex|
  in_project_folder do
    if !File.exist?(output_file_for(cmd))
      capture_output cmd
    end
    if (should == "should")
      output_of(cmd).should match(/#{regex}/)
    else
      output_of(cmd).should_not match(/#{regex}/)
    end
  end
end
