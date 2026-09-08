import { ProfileStorePanel } from "@/plugins/profiles/profile-store-panel";

/**
 * The machine-wide Profile store screen: create and edit configuration
 * profiles, and sync them to a remote. Applying one to a system lives
 * on that system's own Profiles screen instead.
 */
export function ProfileStoreScreen() {
  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-6">
      <header>
        <h1 className="font-semibold text-lg">Profile store</h1>
      </header>
      <ProfileStorePanel />
    </div>
  );
}
