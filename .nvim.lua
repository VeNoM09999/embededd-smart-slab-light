-- You must enable the exrc setting in neovim for this config file to be used.
-- local rust_analyzer = {
-- 	cargo = {
-- 		extraEnv = {
-- 			RUSTUP_TOOLCHAIN = "esp",
-- 		},
-- 		-- target = "x86_64-unknown-linux-gnu",
-- 		target = "xtensa-esp32-none-elf",
-- 		allTargets = false,
-- 	},
-- 	check = {
-- 		command = "check",
-- 	},
-- 	server = {
-- 		extraEnv = {
-- 			RUSTUP_TOOLCHAIN = "esp",
-- 		},
-- 	},
-- }
--
-- -- Note the neovim name of the language server is rust_analyzer with an underscore.
-- vim.lsp.config("rust_analyzer", {
-- 	settings = {
-- 		["rust-analyzer"] = rust_analyzer,
-- 	},
-- })
--
-- vim.lsp.enable("rust_analyzer")
vim.g.rustaceanvim = {
	server = {
        cmd = {
            vim.fn.expand("~/.rustup/toolchains/1.95.0-x86_64-unknown-linux-gnu/bin/rust-analyzer")
            
        },
		default_settings = {
			["rust-analyzer"] = {
				cargo = {
					target = "xtensa-esp32-none-elf",
				},

				check = {
					extraEnv = {
						RUST_BACKTRACE = "1",
						RUSTUP_TOOLCHAIN = "nightly",
					},
				},
			},
		},
	},
}
