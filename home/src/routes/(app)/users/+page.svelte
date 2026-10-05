<script lang="ts">
	/**
	 * Server accounts. Who sees what follows the server's guards
	 * (`crates/graphql/src/{query,mutation}/user.rs`): READ_USERS lists
	 * accounts, MANAGE_USERS creates them and edits other accounts' permissions,
	 * and only the server owner locks, signs out, or deletes. The UI gates are
	 * navigation; the server is the authority, and its refusals surface inline.
	 */
	import { browser } from '$app/environment';
	import { createQuery } from '@tanstack/svelte-query';
	import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';
	import ShieldAlertIcon from '@lucide/svelte/icons/shield-alert';
	import UserPlusIcon from '@lucide/svelte/icons/user-plus';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Avatar, AvatarFallback } from '@stump/ui/components/ui/avatar';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import * as DropdownMenu from '@stump/ui/components/ui/dropdown-menu';
	import { PageHeader } from '@stump/ui/components/ui/page-header';
	import * as Sheet from '@stump/ui/components/ui/sheet';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import * as Table from '@stump/ui/components/ui/table';
	import { request } from '@stump/ui/graphql/client';
	import { errorMessage } from '@stump/ui/utils/errors.js';
	import UserActionDialog, { type UserAction } from '$lib/components/users/UserActionDialog.svelte';
	import UserEditor from '$lib/components/users/UserEditor.svelte';
	import { UsersListDocument } from '$lib/graphql/generated/graphql';
	import { absoluteTime, countNoun, relativeTime } from '$lib/format';
	import { createOidcConfigQuery } from '$lib/oidc';
	import { getHomeSession } from '$lib/session.svelte';
	import {
		OIDC_PERMISSION_SYNC_DOCS,
		canDeleteUser,
		canEditUser,
		canLockOrSignOut,
		canManageUsers,
		canReadUsers,
		sortUsers,
		type ManagedUser
	} from '$lib/users';

	const session = getHomeSession();
	const viewer = $derived(session.user);
	const canRead = $derived(canReadUsers(viewer));
	const canManage = $derived(canManageUsers(viewer));

	const usersQuery = createQuery(() => ({
		queryKey: ['users', canManage],
		queryFn: () => request(UsersListDocument, { manage: canManage }),
		enabled: browser && canRead
	}));
	const users = $derived(sortUsers(usersQuery.data?.users.nodes ?? []));

	// The browser cannot see `STUMP_OIDC_SYNC_PERMISSIONS` or the group
	// mapping, only whether OIDC sign-in is configured at all.
	const oidcQuery = createOidcConfigQuery();
	const oidcEnabled = $derived(Boolean(oidcQuery.data?.enabled));

	const description = $derived(
		viewer?.isServerOwner
			? 'Create accounts, choose what each one may do, and lock, sign out, or delete them.'
			: canManage
				? 'Create accounts and choose what each one may do. Locking, signing out, and deleting accounts stay with the server owner.'
				: 'The accounts on this server and when they last signed in. Changing them needs the Manage users permission.'
	);

	let editorOpen = $state(false);
	let editing = $state.raw<ManagedUser | null>(null);
	let actionOpen = $state(false);
	let action = $state.raw<UserAction | null>(null);

	function openEditor(user: ManagedUser | null): void {
		editing = user;
		editorOpen = true;
	}

	function openAction(kind: UserAction['kind'], user: ManagedUser): void {
		action = { kind, user };
		actionOpen = true;
	}
</script>

<svelte:head>
	<title>Users · Coppice</title>
	<meta name="description" content="Create accounts, set their permissions, and lock or remove them." />
</svelte:head>

{#snippet newUserAction()}
	<Button onclick={() => openEditor(null)}>
		<UserPlusIcon data-icon="inline-start" aria-hidden="true" />
		New user
	</Button>
{/snippet}

<div class="flex flex-col gap-6">
	<PageHeader title="Users" {description} actions={canManage ? newUserAction : undefined} />

	{#if !viewer}
		<Alert role="status">
			<AlertTitle>Checking account access</AlertTitle>
			<AlertDescription>Loading the current account before listing users.</AlertDescription>
		</Alert>
	{:else if !canRead}
		<Alert>
			<AlertTitle>Access required</AlertTitle>
			<AlertDescription>
				Listing accounts needs the server owner or the View users permission (READ_USERS).
			</AlertDescription>
		</Alert>
	{:else}
		{#if canManage && oidcEnabled}
			<Alert role="note">
				<ShieldAlertIcon aria-hidden="true" />
				<AlertTitle>Single sign-on can replace permissions set here</AlertTitle>
				<AlertDescription>
					<p>
						OIDC sign-in is enabled. While OIDC permission sync is on
						(<code>STUMP_OIDC_SYNC_PERMISSIONS</code>, the default) and provider groups are mapped,
						every sign-in replaces an OIDC account’s permissions with its groups’ permissions. That
						affects the accounts marked <strong>OIDC</strong> below, never local ones. Coppice
						does not tell this screen whether sync is on.
						<a
							href={OIDC_PERMISSION_SYNC_DOCS}
							target="_blank"
							rel="noreferrer"
							class="text-primary underline-offset-4 hover:underline"
						>
							How OIDC sync works
						</a>
					</p>
				</AlertDescription>
			</Alert>
		{/if}

		{#if usersQuery.isPending}
			<div class="flex flex-col gap-2" aria-label="Loading users">
				<Skeleton class="h-14 rounded-xl" />
				<Skeleton class="h-14 rounded-xl" />
				<Skeleton class="h-14 rounded-xl" />
			</div>
		{:else if usersQuery.isError}
			<Alert variant="destructive">
				<AlertTitle>Unable to load users</AlertTitle>
				<AlertDescription>{errorMessage(usersQuery.error)}</AlertDescription>
				<Button type="button" variant="outline" class="mt-3 w-fit" onclick={() => usersQuery.refetch()}>
					<RotateCcwIcon data-icon="inline-start" aria-hidden="true" />
					Retry
				</Button>
			</Alert>
		{:else}
			<div class="overflow-hidden rounded-xl border bg-card">
				<Table.Root stacked>
					<Table.Header>
						<Table.Row>
							<Table.Head>Account</Table.Head>
							<Table.Head>Status</Table.Head>
							<Table.Head>Last sign-in</Table.Head>
							<Table.Head class="text-right">Sessions</Table.Head>
							{#if canManage}
								<Table.Head>Permissions</Table.Head>
								<Table.Head class="text-right">Actions</Table.Head>
							{/if}
						</Table.Row>
					</Table.Header>
					<Table.Body>
						{#each users as user (user.id)}
							{@const editable = canEditUser(viewer, user)}
							{@const lockable = canLockOrSignOut(viewer, user)}
							{@const deletable = canDeleteUser(viewer, user)}
							<Table.Row class={user.deletedAt ? 'text-muted-foreground' : undefined}>
								<Table.Cell>
									<div class="flex min-w-0 items-center gap-3">
										<Avatar>
											<AvatarFallback>
												{user.username.trim().charAt(0).toUpperCase() || '?'}
											</AvatarFallback>
										</Avatar>
										<div class="flex min-w-0 flex-col gap-0.5">
											<div class="flex flex-wrap items-center gap-1.5">
												<span class="truncate font-medium">{user.username}</span>
												{#if user.isServerOwner}
													<Badge variant="secondary">Owner</Badge>
												{/if}
												{#if user.id === viewer.id}
													<Badge variant="outline">You</Badge>
												{/if}
												{#if user.oidcEmail}
													<Badge
														variant="outline"
														title={oidcEnabled
															? 'Signs in through OIDC; permission sync may replace its permissions at every sign-in.'
															: 'Signs in through OIDC.'}
													>
														{#if oidcEnabled && canManage}
															<ShieldAlertIcon data-icon="inline-start" aria-hidden="true" />
														{/if}
														OIDC
													</Badge>
												{/if}
											</div>
											<span class="truncate text-xs text-muted-foreground">
												{user.oidcEmail ?? 'Local account'} · joined {relativeTime(user.createdAt)}
											</span>
										</div>
									</div>
								</Table.Cell>
								<Table.Cell data-label="Status">
									{#if user.deletedAt}
										<Badge variant="destructive" title={`Deleted ${absoluteTime(user.deletedAt)}`}>
											Deleted
										</Badge>
									{:else if user.isLocked}
										<Badge variant="destructive">Locked</Badge>
									{:else}
										<Badge variant="outline">Active</Badge>
									{/if}
								</Table.Cell>
								<Table.Cell data-label="Last sign-in">
									<span title={absoluteTime(user.lastLogin)}>{relativeTime(user.lastLogin)}</span>
								</Table.Cell>
								<Table.Cell
									data-label="Sessions"
									class="text-right tabular-nums"
									title={user.maxSessionsAllowed !== null
										? `${user.loginSessionsCount} active of at most ${user.maxSessionsAllowed}`
										: `${user.loginSessionsCount} active, no limit`}
								>
									{user.loginSessionsCount}{#if user.maxSessionsAllowed !== null}&nbsp;/ {user.maxSessionsAllowed}{/if}
								</Table.Cell>
								{#if canManage}
									<Table.Cell data-label="Permissions">
										{#if user.isServerOwner}
											<span class="text-muted-foreground">Every permission</span>
										{:else}
											<span class="inline-flex flex-wrap items-center gap-1.5">
												{countNoun(user.permissions?.length ?? 0, 'permission')}
												{#if user.ageRestriction}
													<Badge
														variant="outline"
														title={user.ageRestriction.restrictOnUnset
															? 'Books rated higher, or not rated at all, are hidden.'
															: 'Books rated higher are hidden.'}
													>
														Rated ≤ {user.ageRestriction.age}
													</Badge>
												{/if}
											</span>
										{/if}
									</Table.Cell>
									<Table.Cell>
										<div class="flex items-center gap-1 @md/table:justify-end">
											{#if editable}
												<Button size="xs" variant="outline" onclick={() => openEditor(user)}>
													<PencilIcon data-icon="inline-start" aria-hidden="true" />
													Edit
												</Button>
											{/if}
											{#if lockable || deletable}
												<DropdownMenu.Root>
													<DropdownMenu.Trigger>
														{#snippet child({ props })}
															<Button
																{...props}
																variant="ghost"
																size="icon-sm"
																aria-label={`More actions for ${user.username}`}
															>
																<EllipsisIcon aria-hidden="true" />
															</Button>
														{/snippet}
													</DropdownMenu.Trigger>
													<DropdownMenu.Content align="end" class="min-w-48">
														{#if lockable}
															<DropdownMenu.Group>
																<DropdownMenu.Item
																	onSelect={() => openAction(user.isLocked ? 'unlock' : 'lock', user)}
																>
																	{user.isLocked ? 'Unlock…' : 'Lock…'}
																</DropdownMenu.Item>
																<DropdownMenu.Item onSelect={() => openAction('signOut', user)}>
																	Sign out everywhere…
																</DropdownMenu.Item>
															</DropdownMenu.Group>
														{/if}
														{#if deletable}
															{#if lockable}
																<DropdownMenu.Separator />
															{/if}
															<DropdownMenu.Group>
																{#if !user.deletedAt}
																	<DropdownMenu.Item
																		variant="destructive"
																		onSelect={() => openAction('delete', user)}
																	>
																		Delete…
																	</DropdownMenu.Item>
																{/if}
																<DropdownMenu.Item
																	variant="destructive"
																	onSelect={() => openAction('purge', user)}
																>
																	Delete permanently…
																</DropdownMenu.Item>
															</DropdownMenu.Group>
														{/if}
													</DropdownMenu.Content>
												</DropdownMenu.Root>
											{/if}
										</div>
									</Table.Cell>
								{/if}
							</Table.Row>
						{/each}
					</Table.Body>
				</Table.Root>
			</div>
			{#if canManage}
				<p class="text-xs text-muted-foreground">
					{viewer.isServerOwner
						? 'The server owner already holds every permission, so the owner account has nothing to edit.'
						: 'The server ignores permission changes to your own account, so another manager or the server owner has to make them. The server owner’s account cannot be edited.'}
				</p>
			{/if}
		{/if}
	{/if}
</div>

<Sheet.Root bind:open={editorOpen}>
	<Sheet.Content class="gap-0 p-0 data-[side=right]:w-full data-[side=right]:sm:max-w-xl">
		<UserEditor user={editing} {oidcEnabled} onsaved={() => (editorOpen = false)} />
	</Sheet.Content>
</Sheet.Root>

<UserActionDialog bind:open={actionOpen} {action} />
