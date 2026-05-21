import { describe, expect, test } from "bun:test";
import { shouldSuppressContextMenu, type ElementDescriptor } from "./context-menu.js";

const div = (classes: string[] = []): ElementDescriptor => ({ tag: "div", classes });

describe("shouldSuppressContextMenu", () => {
  test("suppresses on a bare element with no exempt ancestors", () => {
    expect(shouldSuppressContextMenu([div(), div(), { tag: "body", classes: [] }])).toBe(true);
  });

  test("suppresses when the chain is empty (clicked on non-element)", () => {
    expect(shouldSuppressContextMenu([])).toBe(true);
  });

  test("allows the native menu inside text inputs and textareas", () => {
    expect(shouldSuppressContextMenu([{ tag: "input", classes: [] }])).toBe(false);
    expect(shouldSuppressContextMenu([{ tag: "textarea", classes: [] }])).toBe(false);
  });

  test("allows it inside contenteditable (Tiptap), including nested targets", () => {
    expect(
      shouldSuppressContextMenu([
        { tag: "span", classes: [] },
        { tag: "div", classes: [], contentEditable: "true" },
        { tag: "body", classes: [] },
      ]),
    ).toBe(false);
    // contentEditable="" is the empty-attribute form of true
    expect(shouldSuppressContextMenu([{ tag: "div", classes: [], contentEditable: "" }])).toBe(false);
    // explicit false is not an exemption
    expect(shouldSuppressContextMenu([{ tag: "div", classes: [], contentEditable: "false" }])).toBe(true);
  });

  test("allows it inside Monaco and the terminal", () => {
    expect(shouldSuppressContextMenu([div(["view-line"]), div(["monaco-editor"])])).toBe(false);
    expect(shouldSuppressContextMenu([div(["xterm-rows"]), div(["xterm"])])).toBe(false);
  });
});
