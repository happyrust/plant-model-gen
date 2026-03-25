import { createApp } from 'vue';
import { createVuetify } from 'vuetify';
import 'vuetify/styles';

import App from '@/App.vue';
import router from '@/router';
import '@/styles/main.css';

const vuetify = createVuetify({
  theme: {
    defaultTheme: 'consoleTheme',
    themes: {
      consoleTheme: {
        dark: false,
        colors: {
          primary: '#0f6c5b',
          secondary: '#0b2c3d',
          accent: '#cc7a00',
          background: '#f4efe6',
          surface: '#fffaf2',
          'surface-variant': '#ebe3d5',
          info: '#2f80ed',
          success: '#2e9e5b',
          warning: '#c17c00',
          error: '#b42318',
        },
      },
    },
  },
});

createApp(App).use(vuetify).use(router).mount('#app');
