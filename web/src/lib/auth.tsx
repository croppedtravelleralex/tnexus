"use client";

import { createContext, useCallback, useContext, useEffect, useState } from "react";
import { authApi, healthApi, isApiOfflineError, type User } from "@/lib/api";

const USER_CACHE_KEY = "tnexus_auth_user";

type AuthContextValue = {
  user: User | null;
  /** 首次启动鉴权，仅无缓存用户时阻塞 UI */
  bootstrapping: boolean;
  apiOnline: boolean;
  refresh: () => Promise<void>;
  logout: () => Promise<void>;
};

const AuthContext = createContext<AuthContextValue | null>(null);

function readCachedUser(): User | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = sessionStorage.getItem(USER_CACHE_KEY);
    if (!raw) return null;
    return JSON.parse(raw) as User;
  } catch {
    return null;
  }
}

function writeCachedUser(user: User | null) {
  if (typeof window === "undefined") return;
  try {
    if (user) sessionStorage.setItem(USER_CACHE_KEY, JSON.stringify(user));
    else sessionStorage.removeItem(USER_CACHE_KEY);
  } catch {
    // ignore quota / private mode
  }
}

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const [user, setUser] = useState<User | null>(() => readCachedUser());
  const [bootstrapping, setBootstrapping] = useState(() => readCachedUser() === null);
  const [apiOnline, setApiOnline] = useState(true);

  const refresh = useCallback(async () => {
    const cached = readCachedUser();
    if (!cached) setBootstrapping(true);
    try {
      await healthApi.ping();
      setApiOnline(true);
      const me = await authApi.me();
      setUser(me);
      writeCachedUser(me);
    } catch (err) {
      if (isApiOfflineError(err)) {
        setApiOnline(false);
      }
      setUser(null);
      writeCachedUser(null);
    } finally {
      setBootstrapping(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- mount once
  }, []);

  const logout = useCallback(async () => {
    try {
      await authApi.logout();
    } catch {
      // ignore when API is offline
    }
    setUser(null);
    writeCachedUser(null);
  }, []);

  return (
    <AuthContext.Provider value={{ user, bootstrapping, apiOnline, refresh, logout }}>
      {children}
    </AuthContext.Provider>
  );
}

export function useAuth() {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error("useAuth must be used within AuthProvider");
  return ctx;
}

/** @deprecated 使用 bootstrapping */
export function useAuthLoading() {
  const { bootstrapping } = useAuth();
  return bootstrapping;
}
