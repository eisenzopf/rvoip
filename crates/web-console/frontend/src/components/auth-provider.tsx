import { useState, useCallback, useMemo } from 'react';
import type { ReactNode } from 'react';
import { AuthContext } from '@/hooks/useAuth';
import type { AuthState } from '@/hooks/useAuth';
import { login as apiLogin, logout as apiLogout } from '@/lib/api';

function loadUser(): { id: string | null; username: string; roles: string[] } | null {
  try {
    const raw = localStorage.getItem('rvoip-user');
    if (!raw) return null;
    return JSON.parse(raw) as { id: string | null; username: string; roles: string[] };
  } catch {
    return null;
  }
}

export function AuthProvider({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<{ id: string | null; username: string; roles: string[] } | null>(loadUser);
  const [token, setToken] = useState<string | null>(() => localStorage.getItem('rvoip-token'));

  const loginFn = useCallback(async (username: string, password: string) => {
    const res = await apiLogin({ username, password });
    const userData = { id: res.user.id ?? res.user.user_id ?? null, username: res.user.username, roles: res.user.roles };
    localStorage.setItem('rvoip-token', res.access_token);
    localStorage.setItem('rvoip-refresh-token', res.refresh_token);
    localStorage.setItem('rvoip-user', JSON.stringify(userData));
    setToken(res.access_token);
    setUser(userData);
  }, []);

  const logoutFn = useCallback(() => {
    apiLogout().catch(() => { /* best-effort */ });
    localStorage.removeItem('rvoip-token');
    localStorage.removeItem('rvoip-refresh-token');
    localStorage.removeItem('rvoip-user');
    setToken(null);
    setUser(null);
    window.location.href = '/login';
  }, []);

  const hasRole = useCallback((role: string) => {
    return user?.roles.includes(role) ?? false;
  }, [user]);

  const hasAnyRole = useCallback((roles: string[]) => {
    return roles.some(r => user?.roles.includes(r)) ?? false;
  }, [user]);

  const value: AuthState = useMemo(() => ({
    user,
    token,
    isAuthenticated: !!token && !!user,
    login: loginFn,
    logout: logoutFn,
    hasRole,
    hasAnyRole,
  }), [user, token, loginFn, logoutFn, hasRole, hasAnyRole]);

  return (
    <AuthContext value={value}>
      {children}
    </AuthContext>
  );
}
