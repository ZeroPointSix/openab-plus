import { useMemo } from 'react';
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

export function AdminLayout({ onLogout }: AdminLayoutProps) {
  const location = useLocation();
  const navigate = useNavigate();
  const streamStatus = useSessionStream(true);

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
      navTheme="light"
      fixedHeader
      fixSiderbar
      breakpoint="lg"
      siderWidth={232}
      contentStyle={{ background: '#f0f2f5' }}
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
          <Button type="text" className="account-button">
            <Avatar size="small" icon={<ApiOutlined />} />
            <span>管理员</span>
          </Button>
        </Dropdown>,
      ]}
      token={{
        header: {
          colorBgHeader: '#ffffff',
          colorHeaderTitle: '#17212b',
          heightLayoutHeader: 56,
        },
        sider: {
          colorMenuBackground: '#001529',
          colorTextMenu: 'rgba(255, 255, 255, 0.65)',
          colorTextMenuSelected: '#ffffff',
          colorBgMenuItemSelected: '#1677ff',
          colorTextMenuTitle: '#ffffff',
          colorTextMenuActive: '#ffffff',
        },
        pageContainer: {
          colorBgPageContainer: '#f0f2f5',
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
