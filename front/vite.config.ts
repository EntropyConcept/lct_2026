import preact from '@preact/preset-vite'
import { defineConfig } from 'vite'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
  worker: { format: 'es' },
  server: {
    host: '127.0.0.1',
    proxy: {
      '/api': {
        target: process.env.DISPATCH_API_URL || 'http://127.0.0.1:8080',
        changeOrigin: true,
        configure(proxy) {
          proxy.on('proxyReq', (request, incoming) => {
            // The browser talks to Vite on localhost; keep the backend's
            // same-origin protection intact through the development proxy.
            if (incoming.headers.origin === `http://${incoming.headers.host}`) {
              request.setHeader('origin', new URL(process.env.DISPATCH_API_URL || 'http://127.0.0.1:8080').origin)
            }
          })
        },
      },
    },
  },
  plugins: [
    preact(),
    tailwindcss(),
  ],
})
