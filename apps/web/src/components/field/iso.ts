/** Keyboard movement over the 5×5 field (grid order, row-major 0–24). */

export const COLS = 5;

export function moveCursor(index: number, key: string): number {
  const row = Math.floor(index / COLS);
  const col = index % COLS;
  switch (key) {
    case "ArrowRight":
      return row * COLS + Math.min(COLS - 1, col + 1);
    case "ArrowLeft":
      return row * COLS + Math.max(0, col - 1);
    case "ArrowDown":
      return Math.min(COLS - 1, row + 1) * COLS + col;
    case "ArrowUp":
      return Math.max(0, row - 1) * COLS + col;
    default:
      return index;
  }
}
