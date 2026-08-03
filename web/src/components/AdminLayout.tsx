import { useEffect, useMemo, useState } from 'react';
import {
  ApiOutlined,
  AppstoreOutlined,
  DashboardOutlined,
  LogoutOutlined,
  SettingOutlined,
  UserOutlined,
} from '@ant-design/icons';
import { ProLayout } from '@ant-design/pro-components';
import { Avatar, Badge, Button, Dropdown, Space, Tooltip, Typography } from 'antd';
import { Outlet, useLocation, useNavigate } from 'react-router-dom';
import { clearAdminToken } from '../lib/auth';
import {
  StreamStatus,
  useSessionStream,
} from '../hooks/useSessionStream';

interface AdminLayoutProps {
  onLogout: () => void;
}

const streamLabels: Record<StreamStatus, string> = {
  connecting: '正在连接',
  live: '实时连接',
  reconnecting: '正在重连',
  offline: '离线',
};

const COLLAPSE_STORAGE_KEY = 'openab.admin.sider-collapsed';

function readCollapsedPreference(): boolean {
  try {
    return window.localStorage.getItem(COLLAPSE_STORAGE_KEY) === '1';
  } catch {
    return false;
  }
}

export function AdminLayout({ onLogout }: AdminLayoutProps) {
  const location = useLocation();
  const navigate = useNavigate();
  const streamStatus = useSessionStream(true);
  const [collapsed, setCollapsed] = useState(readCollapsedPreference);

  useEffect(() => {
    try {
      window.localStorage.setItem(COLLAPSE_STORAGE_KEY, collapsed ? '1' : '0');
    } catch {
      // localStorage may be unavailable; collapse state simply won't persist.
    }
  }, [collapsed]);

  const route = useMemo(
    () => ({
      path: '/',
      routes: [
        {
          path: '/overview',
          name: '总览',
          icon: <DashboardOutlined />,
        },
        {
          path: '/sessions',
          name: '会话',
          icon: <AppstoreOutlined />,
        },
        {
          path: '/profiles',
          name: 'Agent Profile',
          icon: <UserOutlined />,
        },
        {
          path: '/config',
          name: 'Gateway 配置',
          icon: <SettingOutlined />,
        },
      ],
    }),
    [],
  );

  const logout = () => {
    clearAdminToken();
    onLogout();
  };

  return (
    <ProLayout
      title="OpenAB Plus"
      logo={<div className="layout-logo">OA</div>}
      layout="mix"
      fixedHeader
      fixSiderbar
      breakpoint="lg"
      siderWidth={232}
      collapsed={collapsed}
      onCollapse={setCollapsed}
      route={route}
      location={{ pathname: location.pathname }}
      menuItemRender={(item, dom) => (
        <button
          type="button"
          className="menu-link"
          onClick={() => item.path && navigate(item.path)}
        >
          {dom}
        </button>
      )}
      avatarProps={false}
      actionsRender={() => [
        <Tooltip title={streamLabels[streamStatus]} key="stream">
          <Space size={6} className="stream-status">
            <Badge
              status={
                streamStatus === 'live'
                  ? 'success'
                  : streamStatus === 'offline'
                    ? 'default'
                    : 'processing'
              }
            />
            <span className="stream-label">{streamLabels[streamStatus]}</span>
          </Space>
        </Tooltip>,
        <Typography.Text type="secondary" key="version" className="version-text">
          v{__APP_VERSION__}
        </Typography.Text>,
        <Dropdown
          key="account"
          menu={{
            items: [
              {
                key: 'logout',
                icon: <LogoutOutlined />,
                label: '退出登录',
                onClick: logout,
              },
            ],
          }}
        >
          <Button type="text" className="account-button" aria-label="账户菜单">
            <Avatar size="small" icon={<ApiOutlined />} />
            <span>管理员</span>
          </Button>
        </Dropdown>,
      ]}
      token={{
        header: {
          colorBgHeader: '#ffffff',
          colorHeaderTitle: '#1b2430',
          heightLayoutHeader: 56,
        },
        sider: {
          colorMenuBackground: '#0e1a28',
          colorTextMenu: '#9db0c6',
          colorTextMenuSecondary: '#7d92a9',
          colorTextMenuSelected: '#ffffff',
          colorBgMenuItemSelected: '#1677ff',
          colorBgMenuItemHover: 'rgba(255, 255, 255, 0.08)',
          colorTextMenuActive: '#ffffff',
          colorBgCollapsedButton: '#0e1a28',
          colorTextCollapsedButton: '#7d92a9',
          colorTextCollapsedButtonHover: '#ffffff',
        },
        pageContainer: {
          paddingInlinePageContainerContent: 24,
          paddingBlockPageContainerContent: 20,
        },
      }}
    >
      <div className="app-content">
        <Outlet />
      </div>
    </ProLayout>
  );
}
