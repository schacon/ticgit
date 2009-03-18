module CliHelpers
  def ti_cmd
    @ti_cmd ||= File.expand_path(File.dirname(__FILE__) + "/../../../../bin/ti")
  end
end

World { |world| world.extend CliHelpers }
