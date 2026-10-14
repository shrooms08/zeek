/** Isometric geometry of the 5×5 field (2:1 diamonds). */

export const COLS = 5;
export const TILE_W = 120;
export const TILE_H = 60;
/** Slab thickness under the field (design: 26 units). */
export const SLAB = 26;

/** Row and column of tile `index` (0–24, row-major). */
export function rowCol(index: number): { row: number; col: number } {
  return { row: Math.floor(index / COLS), col: index % COLS };
}

export function indexOf(row: number, col: number): number {
  return row * COLS + col;
}

/** Center of a tile's top face in field coordinates. */
export function center(index: number): { x: number; y: number } {
  const { row, col } = rowCol(index);
  return { x: ((col - row) * TILE_W) / 2, y: ((col + row) * TILE_H) / 2 };
}

/** The diamond around (x, y) scaled by `s` (1 = full tile), as SVG points. */
export function diamond(x: number, y: number, s = 1): string {
  const w = (TILE_W / 2) * s;
  const h = (TILE_H / 2) * s;
  return `${x},${y - h} ${x + w},${y} ${x},${y + h} ${x - w},${y}`;
}

/** Hole size per depth stage (design: 30 % → 81 % of the tile). */
export const HOLE_SCALE = [0, 0.3, 0.44, 0.58, 0.7, 0.81] as const;
/** Wall depth per stage (design: 4 → 26). */
export const WALL_DEPTH = [0, 4, 8, 13, 19, 26] as const;

/** Moves a selection cursor with the arrow keys (grid order). */
export function moveCursor(index: number, key: string): number {
  const { row, col } = rowCol(index);
  switch (key) {
    case "ArrowRight":
      return indexOf(row, Math.min(COLS - 1, col + 1));
    case "ArrowLeft":
      return indexOf(row, Math.max(0, col - 1));
    case "ArrowDown":
      return indexOf(Math.min(COLS - 1, row + 1), col);
    case "ArrowUp":
      return indexOf(Math.max(0, row - 1), col);
    default:
      return index;
  }
}

/** The SVG viewBox fitting the field and its slab. */
export const VIEWBOX = (() => {
  const halfW = (COLS * TILE_W) / 2;
  const top = -TILE_H / 2 - 40;
  const height = COLS * TILE_H + SLAB + 80;
  return `${-halfW - 20} ${top} ${2 * halfW + 40} ${height}`;
})();
