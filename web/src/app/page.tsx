import Image from "next/image";
import Link from "next/link";
import { ArrowRight, Sparkles } from "lucide-react";
import { Button } from "@/components/ui/button";

const linkBtn =
  "inline-flex items-center justify-center gap-2 rounded-md text-sm font-medium text-shadow-bl-sm shadow-bl-sm transition-colors";

export default function HomePage() {
  return (
    <div className="home-page relative min-h-screen overflow-hidden">
      <div className="home-orb home-orb-a" />
      <div className="home-orb home-orb-b" />
      <div className="home-orb home-orb-c" />

      <header className="relative z-10 border-b px-6 py-4">
        <div className="mx-auto flex max-w-6xl items-center justify-between">
          <Link href="/" className="flex items-center gap-3">
            <div className="home-logo-glow rounded-2xl p-1">
              <Image src="/logo.png" alt="TNexus" width={44} height={44} className="rounded-xl" priority />
            </div>
            <span className="text-shadow-bl text-lg font-semibold text-[#4b4469]">TNexus</span>
          </Link>
          <div className="flex items-center gap-2">
            <Link href="/login" className={`${linkBtn} home-btn-outline h-8 px-3 text-xs`}>
              登录
            </Link>
            <Link href="/studio" className={`${linkBtn} home-btn-primary h-8 px-3 text-xs`}>
              进入工作台
              <ArrowRight className="size-4" />
            </Link>
          </div>
        </div>
      </header>

      <main className="relative z-10 mx-auto max-w-6xl px-6 py-16 md:py-24">
        <div className="mx-auto max-w-3xl text-center">
          <div className="mb-6 inline-flex items-center gap-2 rounded-full border border-[#f9c5d1]/60 bg-white/70 px-4 py-1.5 text-xs font-medium text-[#7a5c6a] shadow-bl-sm backdrop-blur-sm">
            <Sparkles className="size-3.5 text-[#e8a0b4]" />
            AI 创意工作台 · 号池一体化管理
          </div>
          <h1 className="text-shadow-bl text-4xl font-bold tracking-tight text-[#3d3550] md:text-5xl">
            淡粉与奶油色的
            <span className="bg-gradient-to-r from-[#b8a7e0] via-[#f9c5d1] to-[#a0c4ff] bg-clip-text text-transparent">
              {" "}
              生图指挥台
            </span>
          </h1>
          <p className="text-shadow-bl-sm mx-auto mt-5 max-w-xl text-base leading-relaxed text-[#6b6580] md:text-lg">
            TNexus 将导演工作台、号池管理与运维面板整合在同一域名下。登录后即可开始生图，管理员可进入号池与日志管理。
          </p>
          <div className="mt-10 flex flex-wrap items-center justify-center gap-3">
            <Link href="/login" className={`${linkBtn} home-btn-primary h-10 px-6 shadow-bl-md`}>
              立即登录
              <ArrowRight className="size-4" />
            </Link>
            <Link href="/register" className={`${linkBtn} home-btn-outline h-10 px-6`}>
              注册账号
            </Link>
          </div>
        </div>

        <div className="mt-16 grid gap-4 md:grid-cols-3">
          {[
            { title: "导演工作台", desc: "多模型构思、分镜与批量出图，三列布局自动保存。", href: "/studio" },
            { title: "号池管理", desc: "账户列表、调度状态与流水统计。", href: "/accounts" },
            { title: "运维与日志", desc: "运行状态、任务日志与对话调试入口。", href: "/ops" },
          ].map((card) => (
            <Link key={card.title} href={card.href} className="home-card block rounded-2xl p-6 shadow-bl">
              <h2 className="text-shadow-bl text-lg font-semibold text-[#3d3550]">{card.title}</h2>
              <p className="text-shadow-bl-sm mt-2 text-sm leading-relaxed text-[#6b6580]">{card.desc}</p>
            </Link>
          ))}
        </div>
      </main>

      <footer className="relative z-10 border-t border-[#e8e0f4]/80 px-6 py-6 text-center text-xs text-[#9a94a8]">
        TNexus · relai.asia
      </footer>
    </div>
  );
}
