import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { RenameTab } from "@/features/session/rename-tab";
import type { Session } from "@/lib/proto";

function session(over: Partial<Session> & { id: string }): Session {
  return {
    project_id: "proj_1",
    harness: "claude-code",
    workspace: "/home/willie/projects/willie",
    state: { state: "running" },
    created_at: "1",
    clients: 0,
    ...over,
  };
}

describe("RenameTab", () => {
  it("double_click_renames_a_tab_on_enter_and_cancels_on_escape", async () => {
    const user = userEvent.setup();
    const onRename = vi.fn();
    const onSelect = vi.fn();

    render(
      <RenameTab
        session={session({ id: "sess_1", label: "first" })}
        active
        onSelect={onSelect}
        onRename={onRename}
      />,
    );

    await user.dblClick(screen.getByRole("tab", { name: /first/ }));

    const input = screen.getByRole("textbox") as HTMLInputElement;
    expect(input.value).toBe("first");

    await user.clear(input);
    await user.type(input, "renamed{Enter}");

    expect(onRename).toHaveBeenCalledWith("sess_1", "renamed");
    /* No local copy of the label survives the commit: the tab shows
     * the prop's own name again until a fresh snapshot's `sessionName`
     * actually carries the new one. */
    expect(screen.getByRole("tab", { name: /first/ })).toBeDefined();
    expect(screen.queryByRole("textbox")).toBeNull();

    await user.dblClick(screen.getByRole("tab", { name: /first/ }));
    const secondInput = screen.getByRole("textbox") as HTMLInputElement;
    await user.clear(secondInput);
    await user.type(secondInput, "discarded{Escape}");

    expect(onRename).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("tab", { name: /first/ })).toBeDefined();
    expect(screen.queryByRole("textbox")).toBeNull();

    await user.dblClick(screen.getByRole("tab", { name: /first/ }));
    const thirdInput = screen.getByRole("textbox") as HTMLInputElement;
    await user.type(thirdInput, "{Enter}");

    /* Committing the name the session already has is a no-op: no
     * second bridge call for a rename that would not change anything. */
    expect(onRename).toHaveBeenCalledTimes(1);
  });

  it("double_clicking_inside_the_open_input_keeps_the_typed_text", async () => {
    const user = userEvent.setup();
    const onRename = vi.fn();

    render(
      <RenameTab
        session={session({ id: "sess_1", label: "first" })}
        active
        onSelect={vi.fn()}
        onRename={onRename}
      />,
    );

    await user.dblClick(screen.getByRole("tab", { name: /first/ }));
    const input = screen.getByRole("textbox") as HTMLInputElement;
    await user.clear(input);
    await user.type(input, "typed value");

    /* Double-clicking inside the open input to select a word is an
     * ordinary editing gesture, not a request to start editing again —
     * it must never re-seed the value from the session's own name. */
    await user.dblClick(input);

    expect(input.value).toBe("typed value");
    expect(onRename).not.toHaveBeenCalled();
  });

  it("an_emptied_label_is_committed_as_null_to_clear_it", async () => {
    const user = userEvent.setup();
    const onRename = vi.fn();

    render(
      <RenameTab
        session={session({ id: "sess_1", label: "first" })}
        active
        onSelect={vi.fn()}
        onRename={onRename}
      />,
    );

    await user.dblClick(screen.getByRole("tab", { name: /first/ }));
    const input = screen.getByRole("textbox") as HTMLInputElement;
    await user.clear(input);
    await user.type(input, "{Enter}");

    expect(onRename).toHaveBeenCalledWith("sess_1", null);
  });

  it("leading_and_trailing_whitespace_is_trimmed_before_committing", async () => {
    const user = userEvent.setup();
    const onRename = vi.fn();

    render(
      <RenameTab
        session={session({ id: "sess_1", label: "first" })}
        active
        onSelect={vi.fn()}
        onRename={onRename}
      />,
    );

    await user.dblClick(screen.getByRole("tab", { name: /first/ }));
    const input = screen.getByRole("textbox") as HTMLInputElement;
    await user.clear(input);
    await user.type(input, "  new  {Enter}");

    expect(onRename).toHaveBeenCalledWith("sess_1", "new");
  });

  it("blur_commits_like_enter", async () => {
    const user = userEvent.setup();
    const onRename = vi.fn();

    render(
      <RenameTab
        session={session({ id: "sess_1", label: "first" })}
        active
        onSelect={vi.fn()}
        onRename={onRename}
      />,
    );

    await user.dblClick(screen.getByRole("tab", { name: /first/ }));
    const input = screen.getByRole("textbox") as HTMLInputElement;
    await user.clear(input);
    await user.type(input, "blurred");
    fireEvent.blur(input);

    /* Blur commits exactly once — the settled guard that stops Enter's
     * own commit from firing again on the blur that follows it must
     * not swallow a genuine blur-only commit either. */
    expect(onRename).toHaveBeenCalledTimes(1);
    expect(onRename).toHaveBeenCalledWith("sess_1", "blurred");
  });
});
