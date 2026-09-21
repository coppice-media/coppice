<script lang="ts">
	/**
	 * Shared appearance controls. Presets are the primary choice, while the
	 * compact mode group stays at the bottom so both controls keep the same
	 * order in the Home rail and the editor header.
	 */
	import MonitorIcon from "@lucide/svelte/icons/monitor";
	import MoonIcon from "@lucide/svelte/icons/moon";
	import SunIcon from "@lucide/svelte/icons/sun";
	import * as ToggleGroup from "@stump/ui/components/ui/toggle-group/index.js";
	import { cn } from "@stump/ui/utils.js";
	import { uuid } from "@stump/ui/utils/uuid.js";
	import { THEME_PRESETS, theme, type ThemeMode } from "@stump/ui/theme.svelte.js";

	let { class: className }: { class?: string } = $props();

	const MODES: { value: ThemeMode; label: string; icon: typeof SunIcon }[] = [
		{ value: "dark", label: "Dark", icon: MoonIcon },
		{ value: "light", label: "Light", icon: SunIcon },
		{ value: "system", label: "System", icon: MonitorIcon }
	];

	// Several switchers can be mounted at once (the rail and the mobile
	// sheet); radio groups only behave when each has its own name.
	const group = `theme-preset-${uuid()}`;
</script>

<div data-slot="theme-switcher" class={cn(className, "flex w-full flex-col gap-4")}>
	<fieldset data-slot="theme-switcher-presets" class="w-full">
		<legend class="mb-2 text-xs font-medium text-muted-foreground">Palette</legend>
		<div class="grid w-full grid-cols-4 gap-1.5 sm:grid-cols-8">
			{#each THEME_PRESETS as preset (preset.id)}
				<label
					class={cn(
						"group/preset relative flex min-w-0 cursor-pointer items-center justify-center rounded-md border border-transparent p-1.5 transition-colors hover:bg-accent/70",
						preset.kind === "special" && "border-border/70 bg-muted/40"
					)}
					title={`${preset.label}: ${preset.description}`}
				>
					<input
						type="radio"
						name={group}
						value={preset.id}
						checked={theme.preset === preset.id}
						aria-label={`${preset.label} palette — ${preset.description}`}
						onchange={() => (theme.preset = preset.id)}
						class="peer sr-only"
					/>
					<span
						aria-hidden="true"
						style={`--theme-swatch: ${preset.swatch}; background: var(--theme-swatch)`}
						class={cn(
							"block size-6 rounded-full border border-foreground/20 ring-ring/60 ring-offset-2 ring-offset-background transition-[border-color,box-shadow,transform]",
							"peer-checked:scale-110 peer-checked:ring-2 peer-focus-visible:ring-2 peer-focus-visible:ring-ring group-hover/preset:border-foreground/45",
							preset.kind === "special" && "size-7 rounded-md border-foreground/30"
						)}
					></span>
					<span class="sr-only">{preset.label}</span>
				</label>
			{/each}
		</div>
	</fieldset>

	<ToggleGroup.Root
		type="single"
		variant="outline"
		size="sm"
		value={theme.mode}
		onValueChange={(value) => {
			if (value) theme.mode = value as ThemeMode;
		}}
		aria-label="Colour mode"
		class="grid w-full grid-cols-3"
	>
		{#each MODES as mode (mode.value)}
			{@const Icon = mode.icon}
			<ToggleGroup.Item value={mode.value} aria-label={mode.label} class="min-w-0 flex-1 gap-1.5 text-xs">
				<Icon aria-hidden="true" />
				<span class="truncate">{mode.label}</span>
			</ToggleGroup.Item>
		{/each}
	</ToggleGroup.Root>
</div>
