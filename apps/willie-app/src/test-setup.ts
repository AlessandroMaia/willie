/* jsdom lacks the layout APIs the primitives and the theme effect
 * touch. Each stub is inert: it never reports a match, a size or a
 * scroll, so a test that needs one of those installs its own fake. */
if (typeof window.matchMedia !== "function") {
  window.matchMedia = (query: string): MediaQueryList =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    }) as MediaQueryList;
}

if (typeof globalThis.ResizeObserver !== "function") {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

if (typeof Element.prototype.scrollIntoView !== "function") {
  Element.prototype.scrollIntoView = () => {};
}

if (typeof Element.prototype.getAnimations !== "function") {
  Element.prototype.getAnimations = () => [];
}

/* jsdom already defines `window.scrollTo` and `getContext`, but only as
 * a "not implemented" stub that logs a console notice on every call
 * (the router resets the window's scroll on navigation; a canvas
 * measurement runs somewhere in the primitives). There is no absent
 * feature to guard on, so both are replaced outright with a silent,
 * equally inert version. */
window.scrollTo = () => {};

if (typeof HTMLCanvasElement.prototype.getContext === "function") {
  HTMLCanvasElement.prototype.getContext = () => null;
}
