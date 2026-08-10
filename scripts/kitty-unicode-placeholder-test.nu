const script_path = path self

def main [--check] {
    let repo_root = ($script_path | path dirname | path dirname)
    let document = ($repo_root | path join "docs-internal" "kitty-unicode-placeholder-test.md")
    let test_image = ($repo_root | path join "docs" "screenshots" "wezterm-status-powerline.png")

    if not ($document | path exists) or not ($test_image | path exists) {
        error make {
            msg: "Kitty Unicode placeholder test files are missing"
            help: $"Expected ($document) and ($test_image)"
        }
    }

    let nvim_lookup = (mise which nvim | complete)
    if $nvim_lookup.exit_code != 0 {
        error make {
            msg: "mise failed to resolve Neovim"
            help: ($nvim_lookup.stderr | str trim)
        }
    }

    let nvim = ($nvim_lookup.stdout | str trim)
    if $nvim == "" or not ($nvim | path exists) {
        error make {
            msg: "mise did not resolve an installed Neovim executable"
            help: "Run `mise install neovim` and retry"
        }
    }

    let scoop_prefix = (scoop prefix imagemagick | complete)
    if $scoop_prefix.exit_code != 0 {
        error make {
            msg: "ImageMagick is required by Snacks to inspect PNG dimensions"
            help: "Install it with `scoop install imagemagick` and retry"
        }
    }

    let imagemagick_dir = ($scoop_prefix.stdout | str trim)
    let magick = ($imagemagick_dir | path join "magick.exe")
    if not ($magick | path exists) {
        error make {
            msg: $"Scoop reported ImageMagick at ($imagemagick_dir), but magick.exe is missing"
        }
    }
    let launch_path = ($env.PATH | prepend $imagemagick_dir)

    let backslash = (char --unicode 005c)
    let document_for_vim = ($document | str replace --all $backslash '/')
    let test_image_for_vim = ($test_image | str replace --all $backslash '/')
    let configure_inline = (
        "lua assert(Snacks and Snacks.image and Snacks.image.config, "
        + "'Snacks.image is not loaded'); "
        + "Snacks.image.config.doc.inline = true; "
        + "Snacks.image.config.doc.float = false; "
        + "Snacks.image.setup()"
    )
    let verify_environment = (
        "lua local ok, err = xpcall(function() "
        + "local e = Snacks.image.terminal.env(); "
        + "assert(e.supported == true, 'Kitty graphics is not enabled'); "
        + "assert(e.placeholders == true, 'Unicode placeholders are not enabled'); "
        + "local p = _G.kitty_placeholder_test_placement; "
        + "assert(p ~= nil, 'Snacks test placement was not created'); "
        + "local done = vim.wait(5000, function() return p.img.sent or p.img:failed() end, 50); "
        + "assert(done, 'Snacks timed out while processing the test image: ' .. vim.inspect({ "
        + "sent = p.img.sent, failed = p.img:failed(), info = p.img.info, state = p._state, eids = p.eids })); "
        + "assert(not p.img:failed(), p.img._convert and p.img._convert:error() or 'image processing failed'); "
        + "assert(p.img.sent == true, 'Snacks did not transmit the test image'); "
        + "p:update(); "
        + "local rendered = vim.wait(1000, function() return #p.eids > 0 end, 20); "
        + "assert(rendered, 'Snacks did not render the Unicode placeholder grid'); "
        + "assert(#p.eids > 0, 'Snacks did not create any Unicode placeholder image cells'); "
        + "print(vim.inspect({ environment = e, image = p.img.info, placeholder_extmarks = #p.eids })) "
        + "end, debug.traceback); "
        + "if not ok then vim.api.nvim_err_writeln(err); vim.cmd('cquit 1') end"
    )
    let create_test_placement = (
        "lua local p = Snacks.image.placement.new(vim.api.nvim_get_current_buf(), "
        + $"'($test_image_for_vim)', "
        + "{ pos = { 2, 0 }, inline = true, width = 40, height = 6 }); "
        + "p.state = function() return { hidden = false, "
        + "loc = { 2, 0, width = 40, height = 2 }, wins = { vim.api.nvim_get_current_win() } } end; "
        + "_G.kitty_placeholder_test_placement = p"
    )
    let attach_document = "lua Snacks.image.doc.attach(vim.api.nvim_get_current_buf())"

    if not $check {
        print "Launching Neovim with Snacks Unicode placeholders forced on for this process."
        print $"Test document: ($document)"
    }

    with-env {
        PATH: $launch_path
        MAGICK_HOME: $imagemagick_dir
        MAGICK_CONFIGURE_PATH: $imagemagick_dir
        MAGICK_CODER_MODULE_PATH: ($imagemagick_dir | path join "modules" "coders")
        SNACKS_WEZTERM: "false"
        SNACKS_KITTY: "true"
    } {
        if $check {
            ^$nvim --headless -n -c $configure_inline -c $"edit ($document_for_vim)" -c "normal! gg" -c $create_test_placement -c $verify_environment -c "qa!"
        } else {
            ^$nvim -c $configure_inline -c $"edit ($document_for_vim)" -c $attach_document -c "normal! gg"
        }

        let exit_code = ($env.LAST_EXIT_CODE? | default 0)
        if $exit_code != 0 {
            error make {
                msg: $"Neovim exited with status ($exit_code)"
            }
        }
    }
}
