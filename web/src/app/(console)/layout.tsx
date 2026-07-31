import { ConsoleLayout } from "@/components/admin/console-layout";

export default function ConsoleRootLayout({ children }: { children: React.ReactNode }) {
  return <ConsoleLayout>{children}</ConsoleLayout>;
}
