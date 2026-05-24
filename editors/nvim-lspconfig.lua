-- Neovim LSP configuration for the Lace programming language
-- Add this to your Neovim config (init.lua or a dedicated lace.lua file).
--
-- Prerequisites:
--   1. Install lace: cargo install --path crates/lace-cli (or add `lace` to PATH)
--   2. Install nvim-lspconfig: https://github.com/neovim/nvim-lspconfig
--
-- Usage: `lace lsp` is launched automatically when you open a *.lace file.

local lspconfig = require("lspconfig")
local configs = require("lspconfig.configs")

-- Register the lace-lsp server if not already registered
if not configs.lace_lsp then
  configs.lace_lsp = {
    default_config = {
      cmd = { "lace", "lsp" },
      filetypes = { "lace" },
      root_dir = lspconfig.util.root_pattern("lace.toml", ".git"),
      single_file_support = true,
      settings = {},
    },
    docs = {
      description = "Lace Language Server",
      default_config = {
        root_dir = [[root_pattern("lace.toml", ".git")]],
      },
    },
  }
end

lspconfig.lace_lsp.setup({
  on_attach = function(client, bufnr)
    -- Key mappings (adjust to your preferences)
    local opts = { noremap = true, silent = true, buffer = bufnr }
    vim.keymap.set("n", "gd", vim.lsp.buf.definition, opts)
    vim.keymap.set("n", "K",  vim.lsp.buf.hover, opts)
    vim.keymap.set("n", "<leader>f", vim.lsp.buf.format, opts)
    vim.keymap.set("n", "<leader>ca", vim.lsp.buf.code_action, opts)
  end,
  capabilities = require("cmp_nvim_lsp").default_capabilities(),  -- if using nvim-cmp
})

-- Register .lace file type (Neovim does not know about it by default)
vim.filetype.add({
  extension = {
    lace = "lace",
  },
})
