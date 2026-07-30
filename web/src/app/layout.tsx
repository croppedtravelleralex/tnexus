import type { Metadata } from "next";
import { GeistMono } from "geist/font/mono";
import { GeistSans } from "geist/font/sans";
import "./globals.css";
import { ApiStatusBanner } from "@/components/api-status-banner";
import { AuthProvider } from "@/lib/auth";

export const metadata: Metadata = {
  title: "TNexus — AI 视觉创作导演系统",
  description: "导演模式与竞演模式生图工作台",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="zh-CN">
      <body className={`${GeistSans.variable} ${GeistMono.variable} font-sans antialiased`}>
        <AuthProvider>
          <ApiStatusBanner />
          {children}
        </AuthProvider>
      </body>
    </html>
  );
}
