import { vi } from "vitest";

/** jsdom does not calculate layout. Tests can explicitly deliver size/visibility
 * changes; these doubles never pretend to measure real browser geometry. */
export class TestResizeObserver implements ResizeObserver {
  static instances = new Set<TestResizeObserver>();
  readonly targets = new Set<Element>();
  constructor(private callback: ResizeObserverCallback) { TestResizeObserver.instances.add(this); }
  observe(target: Element) { this.targets.add(target); }
  unobserve(target: Element) { this.targets.delete(target); }
  disconnect() { this.targets.clear(); TestResizeObserver.instances.delete(this); }
  static resize(target: Element) {
    for (const observer of this.instances) if (observer.targets.has(target)) {
      observer.callback([{ target, contentRect: target.getBoundingClientRect() } as ResizeObserverEntry], observer);
    }
  }
}
export class TestIntersectionObserver implements IntersectionObserver {
  static instances = new Set<TestIntersectionObserver>();
  readonly targets = new Set<Element>();
  readonly root = null;
  readonly rootMargin = "0px";
  readonly thresholds = [0];
  constructor(private callback: IntersectionObserverCallback) { TestIntersectionObserver.instances.add(this); }
  observe(target: Element) {
    this.targets.add(target);
    // Layout-free jsdom treats everything as visible; virtualization tests
    // that need placeholders install their own observer.
    this.callback([{ target, isIntersecting: true } as IntersectionObserverEntry], this);
  }
  unobserve(target: Element) { this.targets.delete(target); }
  disconnect() { this.targets.clear(); TestIntersectionObserver.instances.delete(this); }
  takeRecords() { return []; }
  static intersect(target: Element, isIntersecting = true) {
    for (const observer of this.instances) if (observer.targets.has(target)) {
      observer.callback([{ target, isIntersecting } as IntersectionObserverEntry], observer);
    }
  }
}
export function installDomMocks() {
  vi.stubGlobal("ResizeObserver", TestResizeObserver);
  vi.stubGlobal("IntersectionObserver", TestIntersectionObserver);
  vi.stubGlobal("matchMedia", vi.fn((query: string): MediaQueryList => ({
    matches: false, media: query, onchange: null,
    addListener: vi.fn(), removeListener: vi.fn(), addEventListener: vi.fn(), removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(() => true),
  })));
  if (!window.PointerEvent) vi.stubGlobal("PointerEvent", MouseEvent);
  Object.defineProperties(Element.prototype, {
    scrollIntoView: { configurable: true, value: vi.fn() },
    scrollTo: { configurable: true, value: vi.fn() },
    hasPointerCapture: { configurable: true, value: vi.fn(() => false) },
    setPointerCapture: { configurable: true, value: vi.fn() },
    releasePointerCapture: { configurable: true, value: vi.fn() },
  });
  Object.defineProperty(document, "fonts", { configurable: true, value: { ready: Promise.resolve() } });
  // jsdom's document is never a focused window. The notification rule asks
  // whether the user can see the app, so tests start "in front" — a test that
  // needs the other side spies on this.
  Object.defineProperty(document, "hasFocus", { configurable: true, value: vi.fn(() => true) });
}
export function resetDomMocks() {
  for (const observer of TestResizeObserver.instances) observer.disconnect();
  for (const observer of TestIntersectionObserver.instances) observer.disconnect();
}
