import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],

  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },

  // Split the bundle so the initial download is just the chat shell.
  // Settings tabs, the paywall, and onboarding are pulled lazily via
  // React.lazy() at the component layer; here we only carve out vendor
  // chunks so a Lucide icon update doesn't bust the React cache.
  build: {
    rollupOptions: {
      output: {
        // Vite 8 ships the rolldown bundler, which only accepts the
        // function form of `manualChunks` (the object form errors with
        // "manualChunks is not a function"). Same vendor split as
        // before — carve stable vendor chunks so e.g. a Lucide icon
        // bump doesn't bust the React cache.
        manualChunks(id) {
          if (!id.includes("node_modules")) return;
          if (/[\\/]node_modules[\\/](react|react-dom|scheduler)[\\/]/.test(id))
            return "vendor-react";
          if (id.includes("/node_modules/@radix-ui/")) return "vendor-radix";
          if (id.includes("/node_modules/lucide-react/")) return "vendor-icons";
          if (id.includes("/node_modules/framer-motion/")) return "vendor-motion";
          if (
            /[\\/]node_modules[\\/](i18next|react-i18next|i18next-browser-languagedetector)[\\/]/.test(
              id
            )
          )
            return "vendor-i18n";
        },
      },
    },
    chunkSizeWarningLimit: 600,
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));
