"use client";

import { createContext, useCallback, useContext, useEffect, useState } from "react";
import { authApi, healthApi, isApiOfflineError, type User } from "@/lib/api";

type AuthContextValue = {
  user: User | null;
  loading: boolean;
  apiOnline: boolean;
  refresh: () => Promise<void>;
  logout: () => Promise<void>;
};

const AuthContext = createContext<AuthContextValue | null>(null);

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const [user, setUser] = useState<User | null>(null);
  const [loading, setLoading] = useState(true);
  const [apiOnline, setApiOnline] = useState(true);

  const refresh = useCallback(async () => {
    try {
      await healthApi.ping();
      setApiOnline(true);
      const me = await authApi.me();
      setUser(me);
    } catch (err) {
      if (isApiOfflineError(err)) {
        setApiOnline(false);
      }
      setUser(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const logout = useCallback(async () => {
    try {
      await authApi.logout();
    } catch {
      // ignore when API is offline
    }
    setUser(null);
  }, []);

  return (
    <AuthContext.Provider value={{ user, loading, apiOnline, refresh, logout }}>
      {children}
    </AuthContext.Provider>
  );
}

export function useAuth() {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error("useAuth must be used within AuthProvider");
  return ctx;
}
