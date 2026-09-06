<script lang="ts">
	import { browser } from '$app/environment';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { createQuery } from '@tanstack/svelte-query';
	import { Toaster } from 'svelte-sonner';
	import { Button } from '@stump/ui/components/ui/button';
	import { Separator } from '@stump/ui/components/ui/separator';
	import { MeDocument } from '@stump/ui/graphql/generated/graphql';
	import { request } from '@stump/ui/graphql/client';
	import { createHomeSession, setHomeSession } from '$lib/session.svelte';

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
		{ href: resolve('/dashboard'), label: 'Dashboard' },
		{ href: resolve('/library'), label: 'Library' },
		{ href: resolve('/reading'), label: 'Reading' },
		{ href: resolve('/devices'), label: 'Devices' },
		{ href: resolve('/account'), label: 'Account' }
	];

	async function logout(): Promise<void> {
		await fetch('/api/v2/auth/logout', { method: 'POST', credentials: 'include' });
		window.location.assign(resolve('/login'));
	}
</script>

<svelte:head>
	<title>Stump</title>
	<meta name="description" content="Your devices, reading activity, and account on this Stump server." />
</svelte:head>

<div class="min-h-screen bg-muted/30">
	<header class="border-b bg-background">
		<div class="mx-auto flex max-w-5xl flex-wrap items-center gap-4 px-4 py-4 lg:px-8">
			<a href={resolve('/dashboard')} class="mr-auto text-lg font-semibold tracking-tight">Stump</a>
			<nav aria-label="Primary navigation" class="flex flex-wrap items-center gap-1 text-sm">
				{#each links as link (link.href)}
					<a
						class="rounded-md px-3 py-2 hover:bg-muted aria-[current=page]:bg-muted aria-[current=page]:font-medium"
						href={link.href}
						aria-current={page.url.pathname.startsWith(link.href) ? 'page' : undefined}
					>
						{link.label}
					</a>
				{/each}
			</nav>
			{#if session.user}
				<span class="hidden text-sm text-muted-foreground md:inline">{session.user.username}</span>
				<Button variant="ghost" size="sm" onclick={logout}>Log out</Button>
			{/if}
		</div>
	</header>
	<Separator />
	<main class="mx-auto max-w-5xl px-4 py-8 lg:px-8">{@render children()}</main>
</div>
<Toaster position="bottom-right" />
