import { useEffect, useMemo, useState } from 'react';
import {
  ApiOutlined,
  AppstoreOutlined,
  DashboardOutlined,
  DownOutlined,
  LogoutOutlined,
  SettingOutlined,
  UserOutlined,
} from '@ant-design/icons';
import { ProLayout } from '@ant-design/pro-components';
import { Avatar, Button, Dropdown, Tooltip } from 'antd';
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
      navTheme="light"
      fixedHeader
      fixSiderbar
      breakpoint="lg"
      siderWidth={236}
      collapsed={collapsed}
      onCollapse={setCollapsed}
      route={route}
      location={{ pathname: location.pathname }}
      headerTitleRender={(logo, _title, props) => (
        <button
          type="button"
          className="header-brand"
          onClick={() => navigate('/overview')}
          aria-label="返回总览"
        >
          {logo}
          {props?.isMobile ? null : (
            <span className="header-brand-text">
              <span className="header-brand-title">OpenAB Plus</span>
              <span className="header-brand-caption">Admin Console</span>
            </span>
          )}
        </button>
      )}
      menuItemRender={(item, dom) => (
        <button
          type="button"
          className="menu-link"
          onClick={() => item.path && navigate(item.path)}
        >
          {dom}
        </button>
      )}
      menuFooterRender={(props) => (
        <div
          className={
            'sider-footer' + (props?.collapsed ? ' is-collapsed' : '')
          }
        >
          <span
            className={'stream-dot is-' + streamStatus}
            aria-hidden="true"
          />
          {props?.collapsed ? null : (
            <span className="sider-footer-text">
              <span className="sider-footer-label">
                {streamLabels[streamStatus]}
              </span>
              <span className="sider-footer-version">
                v{__APP_VERSION__}
              </span>
            </span>
          )}
        </div>
      )}
      avatarProps={false}
      actionsRender={() => [
        <Tooltip
          title={'实时通道：' + streamLabels[streamStatus]}
          key="stream"
        >
          <span className={'stream-status is-' + streamStatus}>
            <span className="stream-dot" aria-hidden="true" />
            <span className="stream-label">
              {streamLabels[streamStatus]}
            </span>
          </span>
        </Tooltip>,
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
            <Avatar className="account-avatar" size="small" icon={<ApiOutlined />} />
            <span className="account-label">管理员</span>
            <DownOutlined className="account-button-caret" />
          </Button>
        </Dropdown>,
      ]}
      token={{
        header: {
          colorBgHeader: '#ffffff',
          colorHeaderTitle: '#1b2430',
          heightLayoutHeader: 64,
        },
        sider: {
          colorMenuBackground: '#ffffff',
          colorTextMenu: '#5b6b7c',
          colorTextMenuSecondary: '#8593a3',
          colorTextMenuSelected: '#1677ff',
          colorBgMenuItemSelected: '#e6f4ff',
          colorBgMenuItemHover: '#f5f8ff',
          colorTextMenuActive: '#1677ff',
          colorBgCollapsedButton: '#ffffff',
          colorTextCollapsedButton: '#8593a3',
          colorTextCollapsedButtonHover: '#1677ff',
        },
        pageContainer: {
          colorBgPageContainer: '#f5f7fa',
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
