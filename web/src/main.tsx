import React from 'react';
import ReactDOM from 'react-dom/client';
import { App as AntApp, ConfigProvider } from 'antd';
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
        token: {
          colorPrimary: '#1677ff',
          colorInfo: '#1677ff',
          colorSuccess: '#52c41a',
          colorWarning: '#faad14',
          colorError: '#ff4d4f',
          colorText: '#1f1f1f',
          colorTextSecondary: '#595959',
          colorTextTertiary: '#8c8c8c',
          colorBgLayout: '#f0f2f5',
          colorBgContainer: '#ffffff',
          colorBgElevated: '#ffffff',
          colorBorder: '#d9d9d9',
          colorBorderSecondary: '#f0f0f0',
          borderRadius: 8,
          boxShadow:
            '0 1px 2px 0 rgba(0, 0, 0, 0.03), 0 1px 6px -1px rgba(0, 0, 0, 0.02), 0 2px 4px 0 rgba(0, 0, 0, 0.02)',
          fontFamily:
            '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif',
        },
        components: {
          Button: {
            controlHeight: 36,
            borderRadius: 6,
          },
          Card: {
            borderRadiusLG: 8,
            colorBgContainer: '#ffffff',
          },
          Table: {
            headerBg: '#fafafa',
            headerColor: '#1f1f1f',
            rowHoverBg: '#f5f9ff',
            borderColor: '#f0f0f0',
          },
          Select: {
            colorBgContainer: '#ffffff',
          },
          Drawer: {
            colorBgElevated: '#ffffff',
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
