/**
 * 正文文本的展示归一化。
 *
 * 背景：抖音 / 小红书文案是按手机屏宽**硬换行**的（每 20 字左右插一个 `\n`），
 * 平台详情页靠窄屏宽掩盖了这一点。搬到桌面端宽容器里用 `whitespace-pre-wrap`
 * 原样渲染，就会出现一列忽长忽短的锯齿短行。
 *
 * 但直接把 `pre-wrap` 去掉也不对：所有换行被折叠成空格，分段信息一起消失。
 *
 * 所以先归一化再交给 `whitespace-pre-line` 渲染：
 *   - 段内单换行 → 合并（中文直接相连，西文之间补一个空格）；
 *   - 段间空行（连续 `\n`）→ 保留，仍然是分段；
 *   - 连续 3 个以上空行压成 1 个空行，避免原文里的空话留白过高。
 */
const CJK_PATTERN = /[\u2e80-\u9fff\uff00-\uffef]/;

const isCjkChar = (char: string): boolean => CJK_PATTERN.test(char);

/** 把 `text` 里的软换行折叠掉，只保留空行分段。空输入返回空串。 */
export const normalizePostBody = (text: unknown): string => {
  const raw = String(text ?? '').replace(/\r\n?/g, '\n');
  if (!raw.trim()) return '';
  return raw
    .split(/\n{2,}/)
    .map((paragraph) => {
      const lines = paragraph
        .split('\n')
        .map((line) => line.trim())
        .filter(Boolean);
      return lines.reduce((acc, line) => {
        if (!acc) return line;
        // 中文之间不需要空格；西文之间必须补，否则 "world" + "this" 会粘成一个词
        const needSpace = !isCjkChar(acc.slice(-1)) && !isCjkChar(line[0]);
        return acc + (needSpace ? ' ' : '') + line;
      }, '');
    })
    .filter(Boolean)
    .join('\n\n');
};
