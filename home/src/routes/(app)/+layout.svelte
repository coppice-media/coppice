<script lang="ts">
	/**
	 * The signed-in shell: a persistent rail from `lg` up, the same nav in a
	 * sheet below it, and a sticky top bar with the account block. Every
	 * screen renders inside `<main>`, which is the `page` container the
	 * dashboard's grid sizes itself against — the rail takes width away from
	 * the content, so a viewport media query would be the wrong ruler.
	 */
	import { browser } from '$app/environment';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { createQuery } from '@tanstack/svelte-query';
	import { Toaster } from 'svelte-sonner';
	import BookOpenIcon from '@lucide/svelte/icons/book-open';
	import CpuIcon from '@lucide/svelte/icons/cpu';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import GaugeIcon from '@lucide/svelte/icons/gauge';
	import HighlighterIcon from '@lucide/svelte/icons/highlighter';
	import InboxIcon from '@lucide/svelte/icons/inbox';
	import LibraryIcon from '@lucide/svelte/icons/library';
	import LinkIcon from '@lucide/svelte/icons/link';
	import LogOutIcon from '@lucide/svelte/icons/log-out';
	import MenuIcon from '@lucide/svelte/icons/menu';
	import PencilRulerIcon from '@lucide/svelte/icons/pencil-ruler';
	import ServerCogIcon from '@lucide/svelte/icons/server-cog';
	import SmartphoneIcon from '@lucide/svelte/icons/smartphone';
	import UserIcon from '@lucide/svelte/icons/user';
	import UsersIcon from '@lucide/svelte/icons/users';
	import { Avatar, AvatarFallback } from '@stump/ui/components/ui/avatar';
	import { Button } from '@stump/ui/components/ui/button';
	import * as Sheet from '@stump/ui/components/ui/sheet';
	import * as Sidebar from '@stump/ui/components/ui/sidebar';
	import { ThemeSwitcher } from '@stump/ui/components/ui/theme-switcher';
	import { MeDocument } from '@stump/ui/graphql/generated/graphql';
	import { request } from '@stump/ui/graphql/client';
	import { createHomeSession, setHomeSession } from '$lib/session.svelte';
	import ShellSearch from '$lib/components/ShellSearch.svelte';
	let { children } = $props();

	const session = createHomeSession();
	setHomeSession(session);
	const meQuery = createQuery(() => ({
		queryKey: ['me'],
		queryFn: () => request(MeDocument, {}),
		enabled: browser
	}));

	$effect(() => {
		if (meQuery.data?.me) session.user = meQuery.data.me;
	});

	const links = [
		{ href: resolve('/dashboard'), label: 'Dashboard', icon: GaugeIcon },
		{ href: resolve('/library'), label: 'Library', icon: LibraryIcon },
		{ href: resolve('/reading'), label: 'Reading', icon: BookOpenIcon },
		{ href: resolve('/annotations'), label: 'Annotations', icon: HighlighterIcon },
		{ href: resolve('/devices'), label: 'Devices', icon: SmartphoneIcon },
		{ href: resolve('/connections'), label: 'Connections', icon: LinkIcon },
		{ href: resolve('/social'), label: 'Social', icon: UsersIcon },
		{ href: resolve('/requests'), label: 'Requests', icon: InboxIcon },
		{ href: resolve('/workers'), label: 'Workers', icon: CpuIcon },
		{ href: resolve('/account'), label: 'Account', icon: UserIcon }
	];

	// The ingest editor is a second app under its own base, outside this
	// one's router; its mutations need `MANAGE_LIBRARY`, so offering the
	// link to anyone else is offering a wall of permission errors.
	const canUseEditor = $derived(
		Boolean(session.user?.isServerOwner || session.user?.permissions.includes('MANAGE_LIBRARY'))
	);
	const canManageComponents = $derived(Boolean(session.user?.isServerOwner));
	const canUseManage = $derived(canUseEditor || canManageComponents);
	const initial = $derived(session.user?.username.trim().charAt(0).toUpperCase() || '?');

	let navOpen = $state(false);

	function isCurrent(href: string): boolean {
		const path = page.url.pathname;
		return path === href || path.startsWith(`${href}/`);
	}

	async function logout(): Promise<void> {
		await fetch('/api/v2/auth/logout', { method: 'POST', credentials: 'include' });
		window.location.assign(resolve('/login'));
	}
</script>

{#snippet nav()}
	<Sidebar.Header class="gap-0 px-3 py-4">
		<a
			href={resolve('/dashboard')}
			onclick={() => (navOpen = false)}
			class="text-lg font-semibold tracking-tight"
		>
			Coppice
		</a>
		<span class="text-xs text-muted-foreground">Your library, devices and reading</span>
	</Sidebar.Header>
	<Sidebar.Content class="px-2">
		<Sidebar.Group>
			<Sidebar.GroupLabel>Browse</Sidebar.GroupLabel>
			<Sidebar.GroupContent>
				<Sidebar.Menu>
					{#each links as link (link.href)}
						{@const Icon = link.icon}
						{@const current = isCurrent(link.href)}
						<Sidebar.MenuItem>
							<Sidebar.MenuButton isActive={current}>
								{#snippet child({ props })}
									<a
										{...props}
										href={link.href}
										aria-current={current ? 'page' : undefined}
										onclick={() => (navOpen = false)}
									>
										<Icon aria-hidden="true" />
										<span>{link.label}</span>
									</a>
								{/snippet}
							</Sidebar.MenuButton>
						</Sidebar.MenuItem>
					{/each}
				</Sidebar.Menu>
			</Sidebar.GroupContent>
		</Sidebar.Group>
		{#if canUseManage}
			<Sidebar.Group>
				<Sidebar.GroupLabel>Manage</Sidebar.GroupLabel>
				<Sidebar.GroupContent>
					<Sidebar.Menu>
						{#if canUseEditor}
							<Sidebar.MenuItem>
								<Sidebar.MenuButton>
									{#snippet child({ props })}
										<a {...props} href="/editor" onclick={() => (navOpen = false)}>
											<PencilRulerIcon aria-hidden="true" />
											<span>Ingest editor</span>
											<ExternalLinkIcon
												aria-hidden="true"
												class="ml-auto size-3.5 text-muted-foreground"
											/>
										</a>
									{/snippet}
								</Sidebar.MenuButton>
							</Sidebar.MenuItem>
						{/if}
						{#if canManageComponents}
							<Sidebar.MenuItem>
								<Sidebar.MenuButton isActive={isCurrent(resolve('/components'))}>
									{#snippet child({ props })}
										<a
											{...props}
											href={resolve('/components')}
											aria-current={isCurrent(resolve('/components')) ? 'page' : undefined}
											onclick={() => (navOpen = false)}
										>
											<ServerCogIcon aria-hidden="true" />
											<span>Components</span>
										</a>
									{/snippet}
								</Sidebar.MenuButton>
							</Sidebar.MenuItem>
						{/if}
					</Sidebar.Menu>
				</Sidebar.GroupContent>
			</Sidebar.Group>
		{/if}
	</Sidebar.Content>
	<Sidebar.Footer class="gap-3 border-t p-3">
		<ThemeSwitcher />
	</Sidebar.Footer>
{/snippet}

<svelte:head>
	<title>Coppice</title>
	<meta
		name="description"
		content="Your devices, reading activity, and account on this Coppice server."
	/>
</svelte:head>

<Sidebar.Provider class="bg-background">
	<a
		href="#main-content"
		class="sr-only z-50 rounded-md bg-primary px-3 py-2 text-sm font-medium text-primary-foreground focus:not-sr-only focus:absolute focus:top-3 focus:left-3"
	>
		Skip to content
	</a>

	<div class="hidden lg:flex">
		<Sidebar.Root collapsible="none" class="sticky top-0 h-svh border-e">
			{@render nav()}
		</Sidebar.Root>
	</div>

	<div class="flex min-w-0 flex-1 flex-col">
		<header
			class="sticky top-0 z-20 flex h-14 shrink-0 items-center gap-2 border-b bg-background/85 px-4 backdrop-blur lg:px-8"
		>
			<Sheet.Root bind:open={navOpen}>
				<Sheet.Trigger>
					{#snippet child({ props })}
						<Button {...props} variant="ghost" size="icon-sm" class="lg:hidden">
							<MenuIcon aria-hidden="true" />
							<span class="sr-only">Open navigation</span>
						</Button>
					{/snippet}
				</Sheet.Trigger>
				<Sheet.Content side="left" class="w-72 gap-0 bg-sidebar p-0 text-sidebar-foreground">
					<Sheet.Header class="sr-only">
						<Sheet.Title>Navigation</Sheet.Title>
						<Sheet.Description>Every screen in the Coppice home app.</Sheet.Description>
					</Sheet.Header>
					{@render nav()}
				</Sheet.Content>
			</Sheet.Root>

			<a href={resolve('/dashboard')} class="font-semibold tracking-tight lg:hidden">Coppice</a>
			<ShellSearch />

			{#if session.user}
				<div class="ml-auto flex items-center gap-2">
					<Avatar>
						<AvatarFallback>{initial}</AvatarFallback>
					</Avatar>
					<span class="hidden text-sm text-muted-foreground sm:inline">{session.user.username}</span>
					<Button variant="ghost" size="sm" onclick={logout}>
						<LogOutIcon data-icon="inline-start" aria-hidden="true" />
						Sign out
					</Button>
				</div>
			{/if}
		</header>

		<main
			id="main-content"
			class="@container/page mx-auto w-full max-w-7xl flex-1 px-4 py-8 lg:px-8"
		>
			{@render children()}
		</main>
	</div>
</Sidebar.Provider>
<Toaster position="bottom-right" />
