import { useState } from 'react';
import { Button, Form, Input, Typography, Alert } from 'antd';
import {
  LockOutlined,
  SafetyCertificateOutlined,
} from '@ant-design/icons';
import { adminApi, ApiError } from '../lib/api';
import { saveAdminToken } from '../lib/auth';

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
      <section className="login-card" aria-labelledby="login-title">
        <div className="login-brand">
          <div className="brand-logo" aria-hidden="true">
            OA
          </div>
          <div>
            <Typography.Title id="login-title" level={2}>
              OpenAB Admin
            </Typography.Title>
            <Typography.Text type="secondary">
              Gateway 运行控制台
            </Typography.Text>
          </div>
        </div>
        <div className="login-security">
          <SafetyCertificateOutlined />
          <span>凭据仅保存在当前浏览器会话</span>
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
    </main>
  );
}
