/// <reference types="vite/client" />

declare module "*.css";

// Injected by vite.config.ts define option
declare const __APP_VERSION__: string;

// Custom Vite env variable (set via import.meta.env or .env file)
interface ImportMetaEnv {
  readonly VITE_RELEASE_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
