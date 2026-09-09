import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { resetForTests, useWorkspacePanel } from "@/store/use-workspace-panel";

beforeEach(resetForTests);

describe("the workspace panel's state", () => {
  it("starts closed on the tree", () => {
    const { result } = renderHook(() => useWorkspacePanel());

    expect(result.current.open).toBe(false);
    expect(result.current.tab).toBe("tree");
  });

  it("opening a file activates the file tab", () => {
    const { result } = renderHook(() => useWorkspacePanel());

    act(() => result.current.openFile("README.md"));

    expect(result.current.file).toBe("README.md");
    expect(result.current.tab).toBe("file");
  });

  it("openAt opens the panel on the tab it names", () => {
    const { result } = renderHook(() => useWorkspacePanel());

    act(() => result.current.openAt("shell"));

    expect(result.current.open).toBe(true);
    expect(result.current.tab).toBe("shell");
  });

  /* The branch and the file describe one workspace; carried across a
   * system change they show the previous system's file under the next
   * system's name. The open flag and the tab are the user's choice of
   * surface, and survive it. */
  it("a system change clears the workspace but keeps the surface", () => {
    const { result } = renderHook(() => useWorkspacePanel());
    act(() => {
      result.current.openAt("file");
      result.current.setBranch("main");
      result.current.openFile("README.md");
    });

    act(() => result.current.forSystem());

    expect(result.current.branch).toBeNull();
    expect(result.current.file).toBeNull();
    expect(result.current.open).toBe(true);
    expect(result.current.tab).toBe("file");
  });
});
