/** Tiny DOM helpers for the dev harness. */

type Attrs = Record<string, string>;
type Child = Node | string;

export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Attrs = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(attrs)) {
    node.setAttribute(name, value);
  }
  node.append(...children);
  return node;
}

export function input(testId: string, placeholder: string, type = "text"): HTMLInputElement {
  return el("input", { "data-testid": testId, placeholder, type });
}

export function button(
  testId: string,
  label: string,
  onClick: () => Promise<void>,
): HTMLButtonElement {
  const b = el("button", { "data-testid": testId, type: "button" }, label);
  b.addEventListener("click", () => {
    b.disabled = true;
    onClick()
      .catch(() => undefined)
      .finally(() => {
        b.disabled = false;
      });
  });
  return b;
}

export function output(testId: string): HTMLPreElement {
  return el("pre", { "data-testid": testId, class: "out" });
}
