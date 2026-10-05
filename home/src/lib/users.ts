import type { UserPermission, UsersListQuery } from '$lib/graphql/generated/graphql';

/**
 * One row of `/users`. `permissions` and `ageRestriction` are only selected
 * for viewers who may manage accounts (see `users.graphql`).
 */
export type ManagedUser = UsersListQuery['users']['nodes'][number];

/** Where the OIDC guide explains group-to-permission synchronization. */
export const OIDC_PERMISSION_SYNC_DOCS =
	'/docs/guides/access-control/oidc#group-to-permission-synchronization';

/** The areas the permission editor groups by, in display order. */
export const PERMISSION_AREAS = [
	{ key: 'account', label: 'Own account' },
	{ key: 'devices', label: 'Devices and sync' },
	{ key: 'files', label: 'Files' },
	{ key: 'libraries', label: 'Libraries' },
	{ key: 'metadata', label: 'Metadata' },
	{ key: 'social', label: 'Book clubs and smart lists' },
	{ key: 'requests', label: 'Requests' },
	{ key: 'email', label: 'Email' },
	{ key: 'notifiers', label: 'Notifiers' },
	{ key: 'jobs', label: 'Jobs and logs' },
	{ key: 'admin', label: 'Users and server' }
] as const;

type PermissionArea = (typeof PERMISSION_AREAS)[number]['key'];

interface PermissionInfo {
	area: PermissionArea;
	label: string;
	description: string;
}

/**
 * Every `UserPermission`, keyed by the generated union so a new server value
 * fails the type check until it is described here. Descriptions follow the
 * enum's doc comments in `crates/models/src/shared/enums.rs`. Within an area
 * the order runs from the narrowest grant to the broadest, which is also the
 * order `explicitPermissions` relies on.
 */
export const PERMISSIONS: Record<UserPermission, PermissionInfo> = {
	CHANGE_PASSWORD: {
		area: 'account',
		label: 'Change own password',
		description: 'Change the password of their own account.'
	},
	CHANGE_USERNAME: {
		area: 'account',
		label: 'Change own username',
		description: 'Rename their own account.'
	},
	CHANGE_AVATAR: {
		area: 'account',
		label: 'Change own avatar',
		description: 'Replace the avatar of their own account.'
	},
	ACCESS_API_KEYS: {
		area: 'account',
		label: 'API keys',
		description: 'Read and create their own API keys.'
	},
	ACCESS_KOREADER_SYNC: {
		area: 'devices',
		label: 'KOReader sync',
		description: 'Sync reading progress with KOReader.'
	},
	ACCESS_KOBO_SYNC: {
		area: 'devices',
		label: 'Kobo sync',
		description: 'Sync with Kobo e-readers.'
	},
	ACCESS_REMOTE_SOURCE: {
		area: 'devices',
		label: 'Remote source',
		description:
			'Act as a source worker: advertise configured roots and serve only explicitly authorized reads.'
	},
	ACCESS_WORKER: {
		area: 'devices',
		label: 'Remote worker',
		description: 'Open the worker socket, claim worker jobs, and upload their outputs.'
	},
	DOWNLOAD_FILE: {
		area: 'files',
		label: 'Download files',
		description: 'Download files from a library.'
	},
	UPLOAD_FILE: {
		area: 'files',
		label: 'Upload files',
		description: 'Upload files to a library.'
	},
	FILE_EXPLORER: {
		area: 'files',
		label: 'File explorer',
		description: 'Browse library folders with the file explorer.'
	},
	EDIT_LIBRARY: {
		area: 'libraries',
		label: 'Edit libraries',
		description: 'Edit the basic details of a library.'
	},
	SCAN_LIBRARY: {
		area: 'libraries',
		label: 'Scan libraries',
		description: 'Scan libraries for new files.'
	},
	EDIT_THUMBNAILS: {
		area: 'libraries',
		label: 'Edit thumbnails',
		description: 'Change the thumbnails of books and series.'
	},
	CREATE_LIBRARY: {
		area: 'libraries',
		label: 'Create libraries',
		description: 'Add new libraries.'
	},
	MANAGE_LIBRARY: {
		area: 'libraries',
		label: 'Manage libraries',
		description: 'Scan, edit, and manage library relations; this is what the ingest editor needs.'
	},
	DELETE_LIBRARY: {
		area: 'libraries',
		label: 'Delete libraries',
		description: 'Delete a library with its books and series.'
	},
	EDIT_METADATA: {
		area: 'metadata',
		label: 'Edit metadata',
		description: 'Edit the database metadata of books and series.'
	},
	WRITE_BACK_METADATA: {
		area: 'metadata',
		label: 'Write metadata to files',
		description: 'Write database metadata back into the files, which can overwrite what the file held.'
	},
	METADATA_FETCH_RECORD_READ: {
		area: 'metadata',
		label: 'View metadata fetches',
		description: 'Read the status of metadata fetches.'
	},
	METADATA_FETCH_RECORD_MANAGE: {
		area: 'metadata',
		label: 'Manage metadata fetches',
		description: 'Act on metadata fetches, such as accepting matches.'
	},
	METADATA_PROVIDER_READ: {
		area: 'metadata',
		label: 'View metadata providers',
		description: 'Read metadata provider configurations.'
	},
	METADATA_PROVIDER_MANAGE: {
		area: 'metadata',
		label: 'Manage metadata providers',
		description: 'Create, update, and delete metadata provider configurations.'
	},
	ACCESS_BOOK_CLUB: {
		area: 'social',
		label: 'Book clubs',
		description: 'Use book clubs.'
	},
	CREATE_BOOK_CLUB: {
		area: 'social',
		label: 'Create book clubs',
		description: 'Start new book clubs.'
	},
	SHARE_BOOK_CLUB_READER: {
		area: 'social',
		label: 'Share guest reading links',
		description:
			'As a club Admin or Creator, issue and manage guest-reader links. Opt-in: it makes library content readable outside Coppice accounts.'
	},
	ACCESS_SMART_LIST: {
		area: 'social',
		label: 'Smart lists',
		description: 'Use, create, and edit smart lists.'
	},
	ACQUIRE_RELEASES: {
		area: 'requests',
		label: 'Acquire releases',
		description: 'Search and grab releases for approved book requests.'
	},
	EMAIL_SEND: {
		area: 'email',
		label: 'Send email',
		description: 'Send email through the configured emailers.'
	},
	EMAIL_ARBITRARY_SEND: {
		area: 'email',
		label: 'Email any address',
		description: 'Send email to any address, bypassing the registered-device requirement.'
	},
	EMAILER_READ: {
		area: 'email',
		label: 'View emailers',
		description: 'Read every emailer configured on the server.'
	},
	EMAILER_CREATE: {
		area: 'email',
		label: 'Create emailers',
		description: 'Add new emailers.'
	},
	EMAILER_MANAGE: {
		area: 'email',
		label: 'Manage emailers',
		description: 'Change existing emailers.'
	},
	READ_NOTIFIER: {
		area: 'notifiers',
		label: 'View notifiers',
		description: 'Read the configured notifiers.'
	},
	CREATE_NOTIFIER: {
		area: 'notifiers',
		label: 'Create notifiers',
		description: 'Add new notifiers.'
	},
	MANAGE_NOTIFIER: {
		area: 'notifiers',
		label: 'Manage notifiers',
		description: 'Change existing notifiers.'
	},
	DELETE_NOTIFIER: {
		area: 'notifiers',
		label: 'Delete notifiers',
		description: 'Remove notifiers.'
	},
	READ_JOBS: {
		area: 'jobs',
		label: 'View jobs',
		description: 'Read the job queue and job history.'
	},
	MANAGE_JOBS: {
		area: 'jobs',
		label: 'Manage jobs',
		description: 'Pause, resume, cancel, or delete jobs.'
	},
	READ_PERSISTED_LOGS: {
		area: 'jobs',
		label: 'View job logs',
		description: 'Read application-level logs such as job logs.'
	},
	READ_SYSTEM_LOGS: {
		area: 'jobs',
		label: 'View system logs',
		description: 'Read the server’s system logs.'
	},
	READ_USERS: {
		area: 'admin',
		label: 'View users',
		description: 'List the accounts on this server and when they last signed in.'
	},
	MANAGE_USERS: {
		area: 'admin',
		label: 'Manage users',
		description:
			'Create accounts and set other accounts’ permissions, age restriction, and session limit. Locking, signing out, and deleting accounts stay with the server owner.'
	},
	MANAGE_SERVER: {
		area: 'admin',
		label: 'Manage server',
		description: 'Manage server settings and features; one step below the server owner.'
	}
};

/** Every permission in display order. */
export const ALL_PERMISSIONS = Object.keys(PERMISSIONS) as UserPermission[];

export const PERMISSION_GROUPS = PERMISSION_AREAS.map((area) => ({
	...area,
	permissions: ALL_PERMISSIONS.filter((permission) => PERMISSIONS[permission].area === area.key)
}));

/**
 * What granting a permission also grants: the server's `AssociatedPermission`
 * table in `crates/models/src/shared/permission_set.rs`, which expands every
 * saved set before storing it. Keep the two in step.
 */
const IMPLIES: Partial<Record<UserPermission, readonly UserPermission[]>> = {
	CREATE_BOOK_CLUB: ['ACCESS_BOOK_CLUB'],
	SHARE_BOOK_CLUB_READER: ['ACCESS_BOOK_CLUB'],
	EMAILER_READ: ['EMAIL_SEND'],
	EMAILER_CREATE: ['EMAILER_READ'],
	EMAILER_MANAGE: ['EMAILER_CREATE', 'EMAILER_READ'],
	EMAIL_ARBITRARY_SEND: ['EMAIL_SEND'],
	CREATE_LIBRARY: ['EDIT_LIBRARY', 'SCAN_LIBRARY'],
	MANAGE_LIBRARY: ['SCAN_LIBRARY', 'EDIT_LIBRARY', 'EDIT_THUMBNAILS'],
	DELETE_LIBRARY: ['MANAGE_LIBRARY'],
	CREATE_NOTIFIER: ['READ_NOTIFIER'],
	MANAGE_NOTIFIER: ['DELETE_NOTIFIER', 'READ_NOTIFIER', 'CREATE_NOTIFIER'],
	DELETE_NOTIFIER: ['MANAGE_NOTIFIER', 'READ_NOTIFIER'],
	MANAGE_USERS: ['READ_USERS'],
	READ_PERSISTED_LOGS: ['READ_JOBS'],
	WRITE_BACK_METADATA: ['EDIT_METADATA']
};

/** `permissions` plus everything they imply, transitively. */
export function withImplied(permissions: Iterable<UserPermission>): Set<UserPermission> {
	const result = new Set<UserPermission>();
	const pending = [...permissions];
	for (let next = pending.pop(); next !== undefined; next = pending.pop()) {
		if (result.has(next)) continue;
		result.add(next);
		pending.push(...(IMPLIES[next] ?? []));
	}
	return result;
}

/**
 * The set to send on save: the ticked permissions and everything they
 * include, in display order. Sending the whole closure stores exactly what
 * the editor showed, however deep an implication chain runs.
 */
export function storedPermissions(granted: Iterable<UserPermission>): UserPermission[] {
	const effective = withImplied(granted);
	return ALL_PERMISSIONS.filter((permission) => effective.has(permission));
}

/**
 * The smallest choice that grows back into `stored`, which is what the editor
 * ticks; the rest shows as included. The server stores the expanded set, so
 * this is how a saved account reads back. Each area is walked from its
 * broadest grant down, so of two permissions that imply each other
 * (`MANAGE_NOTIFIER`, `DELETE_NOTIFIER`) exactly one stays ticked.
 */
export function explicitPermissions(stored: Iterable<UserPermission>): UserPermission[] {
	const explicit = new Set(stored);
	for (const permission of [...ALL_PERMISSIONS].reverse()) {
		if (!explicit.has(permission)) continue;
		explicit.delete(permission);
		if (!withImplied(explicit).has(permission)) explicit.add(permission);
	}
	return ALL_PERMISSIONS.filter((permission) => explicit.has(permission));
}

/** Tick `permission` and drop the ticks it now includes. */
export function grantPermission(
	granted: readonly UserPermission[],
	permission: UserPermission
): UserPermission[] {
	const included = withImplied([permission]);
	return ALL_PERMISSIONS.filter(
		(candidate) =>
			candidate === permission || (granted.includes(candidate) && !included.has(candidate))
	);
}

/** The ticked permissions that bring `permission` along with them. */
export function includedBy(
	granted: readonly UserPermission[],
	permission: UserPermission
): UserPermission[] {
	return granted.filter(
		(source) => source !== permission && withImplied([source]).has(permission)
	);
}

interface Viewer {
	id: string;
	isServerOwner: boolean;
	permissions: readonly string[];
}

/** The `users` query guard: the server owner or `READ_USERS`. */
export function canReadUsers(viewer: Viewer | null | undefined): boolean {
	return Boolean(viewer && (viewer.isServerOwner || viewer.permissions.includes('READ_USERS')));
}

/** The `createUser` and `updateUser` guard: the server owner or `MANAGE_USERS`. */
export function canManageUsers(viewer: Viewer | null | undefined): boolean {
	return Boolean(viewer && (viewer.isServerOwner || viewer.permissions.includes('MANAGE_USERS')));
}

/**
 * Whether `viewer` may edit `target`'s permissions, age restriction, and
 * session limit. `updateUser` ignores those fields on your own account (so
 * nobody grants themselves anything), refuses a server owner's account to
 * non-owners, and an owner already holds every permission.
 */
export function canEditUser(viewer: Viewer, target: ManagedUser): boolean {
	return (
		canManageUsers(viewer) &&
		target.id !== viewer.id &&
		!target.isServerOwner &&
		!target.deletedAt
	);
}

/**
 * `updateUserLockStatus` and `deleteUserSessions` are server-owner-only, and
 * the owner cannot lock themselves. Signing yourself out is not offered here.
 */
export function canLockOrSignOut(viewer: Viewer, target: ManagedUser): boolean {
	return viewer.isServerOwner && target.id !== viewer.id && !target.deletedAt;
}

/** `deleteUser` is server-owner-only and refuses your own and the owner's account. */
export function canDeleteUser(viewer: Viewer, target: ManagedUser): boolean {
	return viewer.isServerOwner && target.id !== viewer.id && !target.isServerOwner;
}

/** Live accounts first with the server owner leading, then by name; deleted ones last. */
export function sortUsers(users: readonly ManagedUser[]): ManagedUser[] {
	return [...users].sort(
		(a, b) =>
			Number(Boolean(a.deletedAt)) - Number(Boolean(b.deletedAt)) ||
			Number(b.isServerOwner) - Number(a.isServerOwner) ||
			a.username.localeCompare(b.username)
	);
}
