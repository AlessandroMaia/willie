import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ProfileStoreScreen } from "@/features/profile-store/profile-store-screen";
import { resetStores } from "@/test-support/reset-stores";

const ipc = vi.hoisted(() => ({
  profiles: {
    list: vi.fn(async () => []),
    create: vi.fn(),
    readFragment: vi.fn(),
    writeFragment: vi.fn(),
    setRemote: vi.fn(),
    push: vi.fn(),
    pull: vi.fn(),
  },
}));

vi.mock("@/lib/ipc", () => ipc);

beforeEach(() => {
  vi.clearAllMocks();
  resetStores();
});

describe("ProfileStoreScreen", () => {
  it("mounts the profile store panel", async () => {
    render(<ProfileStoreScreen />);

    expect(
      await screen.findByRole("heading", { name: "Profile store" }),
    ).toBeDefined();
    expect(screen.getByLabelText("New profile")).toBeDefined();
  });
});
