import { useState } from 'react';
import { Alert, Button, Form, Input, Typography } from 'antd';
import {
  ApiOutlined,
  LockOutlined,
  SafetyCertificateOutlined,
  ThunderboltOutlined,
} from '@ant-design/icons';
import { adminApi, ApiError } from '../lib/api';
import { saveAdminToken } from '../lib/auth';
import openabLogo from '../assets/openab-logo.png?inline';

interface LoginPageProps {
  onAuthenticated: (token: string) => void;
  reason?: string;
}

export function LoginPage({ onAuthenticated, reason }: LoginPageProps) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(reason || '');

  const submit = async ({ token }: { token: string }) => {
    const value = token.trim();
    setLoading(true);
    setError('');
    try {
      await adminApi.probe(value);
      saveAdminToken(value);
      onAuthenticated(value);
    } catch (cause) {
      if (cause instanceof ApiError && cause.status === 503) {
        setError('Gateway 尚未配置 Admin Token，请先完成服务端配置。');
      } else {
        setError('Token 无效或 Gateway 暂时不可用。');
      }
    } finally {
      setLoading(false);
    }
  };

  return (
    <main className="login-page">
      <div className="login-shell">
        <section className="login-hero" aria-label="产品介绍">
          <div className="login-hero-brand">
            <img className="brand-logo" src={openabLogo} alt="" />
            <span>OpenAB Plus</span>
          </div>
          <Typography.Title className="login-hero-title">
            多平台 Agent 会话的运维控制台
          </Typography.Title>
          <Typography.Paragraph className="login-hero-subtitle">
            统一查看 Discord、Slack、Telegram 等平台接入的 Agent
            会话，管理启动 Profile 与 Gateway 配置。
          </Typography.Paragraph>
          <ul className="login-hero-points">
            <li>
              <span className="login-point-icon blue" aria-hidden="true">
                <ApiOutlined />
              </span>
              <div>
                <strong>实时会话流</strong>
                <span>状态事件通过 SSE 即时推送，无需手动刷新</span>
              </div>
            </li>
            <li>
              <span className="login-point-icon amber" aria-hidden="true">
                <ThunderboltOutlined />
              </span>
              <div>
                <strong>Profile 化运行参数</strong>
                <span>按 Agent 类型管理命令、模型与恢复策略</span>
              </div>
            </li>
            <li>
              <span className="login-point-icon green" aria-hidden="true">
                <SafetyCertificateOutlined />
              </span>
              <div>
                <strong>安全的配置管理</strong>
                <span>敏感字段始终脱敏，修改经校验后按策略生效</span>
              </div>
            </li>
          </ul>
        </section>

        <section className="login-card" aria-labelledby="login-title">
          <div className="login-card-heading">
            <Typography.Title id="login-title" level={3}>
              登录控制台
            </Typography.Title>
            <Typography.Text type="secondary">
              输入 Admin Token 继续
            </Typography.Text>
          </div>
          <div className="login-security">
            <SafetyCertificateOutlined />
            <span>凭据保存在当前浏览器，可从 Slack 深链直接打开会话</span>
          </div>
          {error ? (
            <Alert
              type="error"
              showIcon
              message={error}
              className="login-alert"
            />
          ) : null}
          <Form layout="vertical" onFinish={submit} requiredMark={false}>
            <Form.Item
              name="token"
              label="Admin Token"
              rules={[{ required: true, message: '请输入 Admin Token' }]}
            >
              <Input.Password
                autoFocus
                size="large"
                prefix={<LockOutlined />}
                autoComplete="current-password"
                placeholder="输入 Gateway Admin Token"
              />
            </Form.Item>
            <Button
              type="primary"
              htmlType="submit"
              size="large"
              block
              loading={loading}
            >
              登录控制台
            </Button>
          </Form>
          <Typography.Paragraph type="secondary" className="login-footnote">
            该控制台不会发送 Prompt，也不会接管运行中的 Agent。
          </Typography.Paragraph>
        </section>
      </div>
    </main>
  );
}
