// Time-of-day greeting — shared by the in-session empty state and the
// workspace welcome page, so both say the same thing at the same hour
// instead of drifting apart as copies of the same table.

import type { ComponentType } from "react";
import {
  Moon,
  MoonStar,
  Sun,
  SunDim,
  SunMedium,
  Sunrise,
  Sunset,
  type IconProps,
} from "@keyline-icons/react";

export interface Greeting {
  /** The greeting itself, e.g. 「早上好」. */
  label: string;
  /** What follows it — 「，今天想做点什么？」. Part of the greeting so both
   * empty states ask the same thing instead of drifting apart. */
  tail: string;
  Icon: ComponentType<IconProps>;
  /** Icon tint — the only color in the greeting. */
  tone: string;
}

/** The tail every greeting shares. */
const TAIL = "，今天想做点什么？";

export function timeGreeting(): Greeting {
  const h = new Date().getHours();
  if (h < 5) return { label: "夜深了", tail: TAIL, Icon: MoonStar, tone: "text-indigo-400 dark:text-indigo-300" };
  if (h < 9) return { label: "早上好", tail: TAIL, Icon: Sunrise, tone: "text-amber-500 dark:text-amber-400" };
  if (h < 12) return { label: "上午好", tail: TAIL, Icon: Sun, tone: "text-amber-500 dark:text-amber-400" };
  if (h < 14) return { label: "中午好", tail: TAIL, Icon: SunMedium, tone: "text-orange-500 dark:text-orange-400" };
  if (h < 18) return { label: "下午好", tail: TAIL, Icon: SunDim, tone: "text-amber-600 dark:text-amber-500" };
  if (h < 20) return { label: "傍晚好", tail: TAIL, Icon: Sunset, tone: "text-orange-500 dark:text-orange-400" };
  return { label: "晚上好", tail: TAIL, Icon: Moon, tone: "text-indigo-400 dark:text-indigo-300" };
}
