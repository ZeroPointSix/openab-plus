import { useEffect, useState } from 'react';
import { Navigate, Route, Routes, useLocation, useNavigate } from 'react-router-dom';
import { AdminLayout } from './components/AdminLayout';
import { LoginPage } from './pages/LoginPage';
import { OverviewPage } from './pages/OverviewPage';
import { SessionWorkbenchPage } from './pages/SessionWorkbenchPage';
import { ProfilesPage } from './pages/ProfilesPage';
import { ConfigPage } from './pages/ConfigPage';
import { WorkspacePage } from './pages/WorkspacePage';
import {
  loginPathFor,
  readAdminToken,
  returnToFromSearch,
  UNAUTHORIZED_EVENT,
} from './lib/auth';

export function App() {
  const location = useLocation();
  const navigate = useNavigate();
  const [token, setToken] = useState(readAdminToken);
  const [loginReason, setLoginReason] = useState('');

  useEffect(() => {
    const unauthorized = () => {
      setLoginReason('登录已失效，请重新输入 Admin Token。');
      setToken('');
    };
    window.addEventListener(UNAUTHORIZED_EVENT, unauthorized);
    return () => window.removeEventListener(UNAUTHORIZED_EVENT, unauthorized);
  }, []);

  if (!token) {
    if (location.pathname !== '/login') {
      return <Navigate to={loginPathFor(location)} replace />;
    }

    return (
      <LoginPage
        reason={loginReason}
        onAuthenticated={(value) => {
          setLoginReason('');
          setToken(value);
          navigate(returnToFromSearch(location.search), { replace: true });
        }}
      />
    );
  }

  if (location.pathname === '/login') {
    return (
      <Navigate to={returnToFromSearch(location.search)} replace />
    );
  }

  return (
    <Routes>
      <Route
        element={
          <AdminLayout
            onLogout={() => {
              setLoginReason('');
              setToken('');
            }}
          />
        }
      >
        <Route index element={<Navigate to="/overview" replace />} />
        <Route path="/overview" element={<OverviewPage />} />
        <Route path="/sessions" element={<SessionWorkbenchPage />} />
        <Route path="/sessions/:id" element={<SessionWorkbenchPage />} />
        <Route path="/profiles" element={<ProfilesPage />} />
        <Route path="/workspace" element={<WorkspacePage />} />
        <Route path="/config" element={<ConfigPage />} />
        <Route path="*" element={<Navigate to="/overview" replace />} />
      </Route>
    </Routes>
  );
}
