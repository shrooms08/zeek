/** QR codes for payment requests. */

import QRCode from "qrcode";

/** Renders `text` as an SVG QR code (error correction M, quiet zone 2). */
export async function qrSvg(text: string): Promise<string> {
  return QRCode.toString(text, { type: "svg", errorCorrectionLevel: "M", margin: 2 });
}
