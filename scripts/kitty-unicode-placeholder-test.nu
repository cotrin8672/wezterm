const script_path = path self

def main [--check] {
    let repo_root = ($script_path | path dirname | path dirname)
    let document = ($repo_root | path join "docs-internal" "kitty-unicode-placeholder-test.md")

    if not ($document | path exists) {
        error make {
            msg: $"Kitty Unicode placeholder test document is missing: ($document)"
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

    let document_for_vim = ($document | str replace --all '\\' '/')
    let configure_inline = (
        "lua assert(Snacks and Snacks.image and Snacks.image.config, "
        + "'Snacks.image is not loaded'); "
        + "Snacks.image.config.doc.inline = true; "
        + "Snacks.image.config.doc.float = false"
    )
    let verify_environment = (
        "lua local e = Snacks.image.terminal.env(); "
        + "assert(e.supported == true, 'Kitty graphics is not enabled'); "
        + "assert(e.placeholders == true, 'Unicode placeholders are not enabled'); "
        + "print(vim.inspect(e))"
    )

    if not $check {
        print "Launching Neovim with Snacks Unicode placeholders forced on for this process."
        print $"Test document: ($document)"
    }

    with-env {
        SNACKS_WEZTERM: "false"
        SNACKS_KITTY: "true"
    } {
        if $check {
            ^$nvim --headless -c $configure_inline -c $"edit ($document_for_vim)" -c $verify_environment -c "qa!"
        } else {
            ^$nvim -c $configure_inline -c $"edit ($document_for_vim)"
        }

        let exit_code = ($env.LAST_EXIT_CODE? | default 0)
        if $exit_code != 0 {
            error make {
                msg: $"Neovim exited with status ($exit_code)"
            }
        }
    }
}
