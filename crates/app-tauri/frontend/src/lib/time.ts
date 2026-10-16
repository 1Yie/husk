/** Relative age of a unix-seconds stamp. Clock skew (a stamp in the future)
 * and pre-epoch zeroes both collapse to 「刚刚」 rather than a negative age. */
export function timeAgo(seconds: number): string {
  if (!seconds) return "";
  const elapsed = Date.now() / 1000 - seconds;
  if (elapsed < 60) return "刚刚";
  const minutes = elapsed / 60;
  if (minutes < 60) return `${Math.floor(minutes)} 分钟前`;
  const hours = minutes / 60;
  if (hours < 24) return `${Math.floor(hours)} 小时前`;
  const days = hours / 24;
  if (days < 30) return `${Math.floor(days)} 天前`;
  return new Date(seconds * 1000).toLocaleDateString();
}
