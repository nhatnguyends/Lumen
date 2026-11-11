import { build } from 'vite'
await build({ configFile: false, build: {
  ssr: 'widgets/music/adapter/nativeMusicBridge.ts', outDir: 'native-dock/target/debug', emptyOutDir: false,
  rollupOptions: { external: ['koffi'], output: { entryFileNames: 'music-bridge.cjs', format: 'cjs' } }
} })
