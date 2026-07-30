"use client";

import { motion } from "framer-motion";
import Image from "next/image";
import Link from "next/link";
import { SiteHeader } from "@/components/site-header";
import { Button } from "@/components/ui/button";

export default function HomePage() {
  return (
    <div className="home-page min-h-screen">
      <SiteHeader variant="home" />
      <main className="mx-auto flex max-w-5xl flex-col gap-12 px-6 py-14 md:py-20">
        <motion.section
          initial={{ opacity: 0, y: 24 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ type: "spring", stiffness: 300, damping: 30 }}
          className="flex flex-col items-center gap-8 text-center md:items-start md:text-left"
        >
          <div className="flex flex-col items-center gap-5 md:flex-row md:items-center">
            <div className="rounded-3xl bg-white/60 p-2 shadow-[0_12px_40px_rgba(184,167,224,0.35)] ring-1 ring-white/80 backdrop-blur-sm">
              <Image
                src="/logo.png"
                alt="TNexus"
                width={96}
                height={96}
                className="rounded-2xl"
                priority
              />
            </div>
            <div className="space-y-2">
              <p className="text-sm font-medium uppercase tracking-[0.24em] text-[#8b7cc8]">
                AI Visual Director
              </p>
              <h1 className="bg-gradient-to-r from-[#5c4f8f] via-[#7b6cb0] to-[#6b9fd4] bg-clip-text text-4xl font-semibold leading-tight text-transparent md:text-5xl">
                TNexus
              </h1>
            </div>
          </div>

          <div className="max-w-3xl space-y-5">
            <h2 className="text-3xl font-semibold leading-snug text-[#3d3566] md:text-4xl">
              用导演思维组织创意，用竞演模式探索风格
            </h2>
            <p className="text-lg leading-relaxed text-[#6b6580]">
              TNexus 是独立的 AI 生图工作台：双层创意因子、A/B 工作流路径、ChatGPT 与 Grok 双引擎，资产通过 R2 CDN 分发。
            </p>
          </div>

          <div className="flex flex-wrap justify-center gap-3 md:justify-start">
            <Link href="/register">
              <Button size="lg" className="home-btn-primary h-11 border-0 px-7">
                免费注册
              </Button>
            </Link>
            <Link href="/studio">
              <Button size="lg" className="home-btn-outline h-11 px-7">
                进入工作台
              </Button>
            </Link>
          </div>
        </motion.section>

        <div className="grid gap-4 md:grid-cols-3">
          {[
            { title: "导演模式", desc: "单一审美大脑定方向，再执行生图" },
            { title: "竞演模式", desc: "ChatGPT 与 Grok 并行出图对比" },
            { title: "双层因子", desc: "导演因子 + 画面质感二维可视化调节" },
          ].map((item, i) => (
            <motion.div
              key={item.title}
              initial={{ opacity: 0, y: 16 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.1 * i, type: "spring", stiffness: 300, damping: 30 }}
            >
              <div className="home-card p-6">
                <h3 className="mb-2 text-lg font-medium text-[#3d3566]">{item.title}</h3>
                <p className="text-sm leading-relaxed text-[#6b6580]">{item.desc}</p>
              </div>
            </motion.div>
          ))}
        </div>
      </main>
    </div>
  );
}
