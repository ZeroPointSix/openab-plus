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
          colorBgLayout: '#f3f5f9',
          colorBgContainer: '#ffffff',
          colorBgElevated: '#ffffff',
          colorBorder: '#dbe3ec',
          colorBorderSecondary: '#e8eef4',
          colorFillSecondary: '#eef2f7',
          borderRadius: 8,
          boxShadow:
            '0 1px 2px 0 rgba(16, 27, 41, 0.04), 0 1px 6px -1px rgba(16, 27, 41, 0.03), 0 2px 4px 0 rgba(16, 27, 41, 0.03)',
          fontFamily:
            'Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif',
        },
        components: {
          Button: {
            controlHeight: 34,
            borderRadius: 8,
          },
          Card: {
            borderRadiusLG: 12,
            colorBgContainer: '#ffffff',
          },
          Table: {
            headerBg: '#f7f9fc',
            headerColor: '#45536a',
            rowHoverBg: '#f5f8ff',
            borderColor: '#e8eef4',
            headerSplitColor: 'transparent',
          },
          Select: {
            colorBgContainer: '#ffffff',
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
