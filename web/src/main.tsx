import React from 'react';
import ReactDOM from 'react-dom/client';
import { App as AntApp, ConfigProvider, theme } from 'antd';
import zhCN from 'antd/locale/zh_CN';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { HashRouter } from 'react-router-dom';
import 'antd/dist/reset.css';
import './styles.css';
import { App } from './App';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 15_000,
      retry: 1,
      refetchOnWindowFocus: false,
    },
    mutations: {
      retry: 0,
    },
  },
});

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ConfigProvider
      locale={zhCN}
      theme={{
        algorithm: theme.defaultAlgorithm,
        token: {
          colorPrimary: '#1677ff',
          colorInfo: '#1677ff',
          colorSuccess: '#1f9d68',
          colorWarning: '#d98b16',
          colorError: '#d14343',
          colorText: '#1b2430',
          colorTextSecondary: '#5b6b7c',
          colorTextTertiary: '#8593a3',
          colorBgLayout: '#f4f6fa',
          colorBorder: '#d9e2ec',
          colorBorderSecondary: '#e8eef4',
          colorFillSecondary: '#eef2f7',
          borderRadius: 8,
          fontFamily:
            'Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
        },
        components: {
          Button: {
            controlHeight: 34,
          },
          Card: {
            borderRadiusLG: 10,
          },
          Table: {
            headerBg: '#f7f9fc',
            headerColor: '#45536a',
            rowHoverBg: '#f5f8ff',
            headerSplitColor: 'transparent',
          },
          Select: {
            optionSelectedBg: '#e8f1ff',
          },
          Drawer: {
            colorBgElevated: '#ffffff',
          },
          DatePicker: {
            colorBgElevated: '#ffffff',
          },
          Tooltip: {
            colorBgSpotlight: '#1b2430',
          },
          Menu: {
            darkItemBg: '#0e1a28',
            darkSubMenuItemBg: '#0e1a28',
            darkPopupBg: '#0e1a28',
            darkItemColor: '#9db0c6',
            darkItemHoverColor: '#ffffff',
            darkItemHoverBg: 'rgba(255, 255, 255, 0.08)',
            darkItemSelectedBg: '#1677ff',
            darkItemSelectedColor: '#ffffff',
          },
          Layout: {
            siderBg: '#0e1a28',
            triggerBg: '#0e1a28',
            triggerColor: '#7d92a9',
          },
        },
      }}
    >
      <AntApp>
        <QueryClientProvider client={queryClient}>
          <HashRouter>
            <App />
          </HashRouter>
        </QueryClientProvider>
      </AntApp>
    </ConfigProvider>
  </React.StrictMode>,
);
