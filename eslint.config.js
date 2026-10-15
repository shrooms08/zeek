import js from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";

export default tseslint.config(
  {
    ignores: [
      "**/dist/**",
      "**/node_modules/**",
      "packages/zeek-pocket/wasm/**",
      "target/**",
      "crates/**",
      "vendor/**",
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.strictTypeChecked,
  {
    languageOptions: {
      parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname },
    },
    rules: {
      "@typescript-eslint/restrict-template-expressions": ["error", { allowNumber: true }],
    },
  },
  {
    files: ["**/*.js", "**/*.mjs", "**/*.d.mts"],
    ...tseslint.configs.disableTypeChecked,
  },
  {
    files: ["apps/harness/e2e/**/*.mjs", "scripts/**/*.mjs"],
    languageOptions: { globals: { ...globals.node } },
  },
);
