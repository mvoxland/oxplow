import { afterEach, expect, test } from "bun:test";
import { useEffect } from "react";
import { cleanup, render } from "@testing-library/react";

// Proves the happy-dom + @testing-library/react harness works end to
// end: a real React component renders into a real DOM, queries resolve,
// and unmount runs effect cleanup. This guards the test infrastructure
// itself — if happy-dom or testing-library breaks, this fails loudly
// rather than every future component test failing mysteriously. The
// App.tsx / EditorPane / TerminalPane teardown tests build on this.
afterEach(cleanup);

function Greeting({ name }: { name: string }) {
  return <div data-testid="greeting">Hello {name}</div>;
}

test("renders a React component into a real DOM", () => {
  const { getByTestId } = render(<Greeting name="oxplow" />);
  expect(getByTestId("greeting").textContent).toBe("Hello oxplow");
});

test("unmount tears the component out of the DOM", () => {
  const { unmount } = render(<Greeting name="bye" />);
  expect(document.body.textContent).toContain("Hello bye");
  unmount();
  expect(document.body.textContent).toBe("");
});

test("effect cleanup runs on unmount (no leaked subscriptions)", () => {
  let mounted = false;
  function WithEffect() {
    useEffect(() => {
      mounted = true;
      return () => {
        mounted = false;
      };
    }, []);
    return <span>effect</span>;
  }

  const { unmount } = render(<WithEffect />);
  expect(mounted).toBe(true);
  unmount();
  expect(mounted).toBe(false);
});
