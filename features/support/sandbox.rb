module SandboxHelpers
  def setup_safe_folder
    FileUtils.rm_rf   @tmp_root = File.expand_path(File.dirname(__FILE__) + "/../../tmp")
    FileUtils.mkdir_p @tmp_root
    FileUtils.mkdir_p @home_path = File.expand_path(File.join(@tmp_root, "home"))
    ENV['HOME'] = @home_path
    FileUtils.mkdir_p @project_path = File.expand_path(File.join(@tmp_root, "project"))
    @lib_path = File.expand_path(File.dirname(__FILE__) + '/../../lib')
  end

  def in_tmp_folder(&block)
    FileUtils.chdir(@tmp_root, &block)
  end

  def in_project_folder(&block)
    FileUtils.chdir(@project_path, &block)
  end

  def in_home_folder(&block)
    FileUtils.chdir(@home_path, &block)
  end

  def output_file_for(cmd)
    File.expand_path(File.join(@tmp_root, cmd.gsub(/[^A-Za-z0-9]/, '_') << ".out"))
  end
  
  def capture_output(cmd)
    stdout = output_file_for(cmd)
    system "#{cmd} > #{stdout} 2> #{stdout}"
  end
  
  def output_of(cmd)
    output_file = output_file_for(cmd)
    File.read(output_file)
  end

end

World { |world| world.extend SandboxHelpers }
