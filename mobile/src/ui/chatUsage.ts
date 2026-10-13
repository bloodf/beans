import type { ChatUsage } from "../core/model";
import { language } from "../i18n";

// The phone has no usage formatter. These rows use only totals reported by the core.
export function chatUsageRows(usage: ChatUsage) {
  const zh = language === "zh";
  const tokens = usage.context_tokens.toLocaleString(zh ? "zh-CN" : "en-US");
  const context = usage.context_window > 0
    ? `${tokens} / ${usage.context_window.toLocaleString(zh ? "zh-CN" : "en-US")} · ${Math.round(usage.context_tokens / usage.context_window * 100)}%`
    : `${tokens} ${zh ? "词元" : "tokens"}`;
  const validCost = Number.isFinite(usage.cost_usd) && usage.cost_usd >= 0;
  const dollars = usage.cost_usd > 0 && usage.cost_usd < 0.01 ? "<$0.01" : `$${usage.cost_usd.toFixed(2)}`;
  const cost = validCost && usage.cost_known === true ? dollars
    : validCost && usage.cost_usd > 0
      ? `${dollars} ${zh ? "已知小计 · 总额未知" : "known subtotal · total unknown"}`
      : zh ? "费用未知" : "Unknown cost";
  const turns = zh ? `${usage.turns} 轮` : `${usage.turns} ${usage.turns === 1 ? "turn" : "turns"}`;
  return [
    { title: zh ? "上下文" : "Context", subtitle: context },
    { title: zh ? "已花费" : "Spent", subtitle: `${cost} · ${turns}` },
  ];
}
