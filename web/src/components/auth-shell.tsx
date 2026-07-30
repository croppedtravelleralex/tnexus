"use client";

import Image from "next/image";
import Link from "next/link";
import { motion } from "framer-motion";

type Props = {
  title: string;
  subtitle?: string;
  children: React.ReactNode;
  footer: React.ReactNode;
};

export function AuthShell({ title, subtitle, children, footer }: Props) {
  return (
    <div className="auth-page relative flex min-h-screen items-center justify-center overflow-hidden px-4 py-10">
      <div className="auth-orb auth-orb-a" />
      <div className="auth-orb auth-orb-b" />

      <motion.div
        initial={{ opacity: 0, y: 24, scale: 0.98 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={{ type: "spring", stiffness: 280, damping: 28 }}
        className="auth-card relative z-10 w-full max-w-md p-8 md:p-10"
      >
        <div className="mb-8 flex flex-col items-center text-center">
          <div className="auth-logo-ring mb-5 p-1">
            <Image
              src="/logo.png"
              alt="TNexus"
              width={88}
              height={88}
              className="rounded-2xl"
              priority
            />
          </div>
          <p className="text-xs font-medium uppercase tracking-[0.28em] text-violet-300/80">
            AI Creative Intelligence
          </p>
          <h1 className="mt-2 bg-gradient-to-r from-white via-violet-100 to-cyan-200 bg-clip-text text-2xl font-semibold text-transparent">
            {title}
          </h1>
          {subtitle && <p className="mt-2 text-sm text-zinc-400">{subtitle}</p>}
        </div>

        {children}

        <div className="mt-6 text-center text-sm text-zinc-400">{footer}</div>
      </motion.div>

      <Link
        href="/"
        className="absolute left-6 top-6 z-10 flex items-center gap-2 text-sm text-zinc-400 transition hover:text-white"
      >
        <Image src="/logo.png" alt="" width={28} height={28} className="rounded-md" />
        TNexus
      </Link>
    </div>
  );
}
