import type { CSSProperties } from "react";

const MATRIX_SIZE = 5;
const CELL_COUNT = MATRIX_SIZE * MATRIX_SIZE;

function buildSpiralOrder() {
  const order = new Array<number>(CELL_COUNT);
  let top = 0;
  let bottom = MATRIX_SIZE - 1;
  let left = 0;
  let right = MATRIX_SIZE - 1;
  let step = 0;

  while (top <= bottom && left <= right) {
    for (let column = left; column <= right; column += 1) order[top * MATRIX_SIZE + column] = step++;
    for (let row = top + 1; row <= bottom; row += 1) order[row * MATRIX_SIZE + right] = step++;
    if (top < bottom) {
      for (let column = right - 1; column >= left; column -= 1) order[bottom * MATRIX_SIZE + column] = step++;
    }
    if (left < right) {
      for (let row = bottom - 1; row > top; row -= 1) order[row * MATRIX_SIZE + left] = step++;
    }
    top += 1;
    bottom -= 1;
    left += 1;
    right -= 1;
  }

  return order;
}

const SPIRAL_ORDER = buildSpiralOrder();
const DOTS = SPIRAL_ORDER.map((order) => ({
  order,
  staticOpacity: 0.16 + (order / (CELL_COUNT - 1)) * 0.78,
}));

export function RedetectionLoader() {
  return (
    <span className="redetection-loader" aria-hidden="true" data-testid="redetection-loader">
      {DOTS.map(({ order, staticOpacity }) => (
        <span
          key={order}
          style={{
            "--redetection-order": order,
            "--redetection-static-opacity": staticOpacity,
          } as CSSProperties}
        />
      ))}
    </span>
  );
}
