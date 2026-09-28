import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // Packaging creates an Applications symlink under target/. Never scan build artifacts.
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
  },
});
