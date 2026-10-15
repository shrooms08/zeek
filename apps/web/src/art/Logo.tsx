/** <Logo>: the Z mark, or the lockup with "eek" at the configured slant. */

import { EEK_SLANT, eekMarkup, LOGO_VIEWBOX, zMarkMarkup, type EekSlant } from "./logoGeometry";

export function Logo({
  variant = "lockup",
  height = 40,
  fg = "#FFFFFF",
  bg = "#0D0D0D",
  slant = EEK_SLANT,
  label = "Zeek",
}: {
  variant?: "mark" | "lockup";
  height?: number;
  fg?: string;
  bg?: string;
  slant?: EekSlant;
  label?: string | null;
}) {
  const html = zMarkMarkup(fg, bg) + (variant === "lockup" ? eekMarkup(bg, slant) : "");
  return (
    <svg
      viewBox={LOGO_VIEWBOX}
      height={height}
      width={(height * 483) / 358}
      role={label === null ? undefined : "img"}
      aria-label={label ?? undefined}
      aria-hidden={label === null ? true : undefined}
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}
