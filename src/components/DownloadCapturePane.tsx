import * as Label from "@radix-ui/react-label";
import * as Select from "@radix-ui/react-select";
import { AlertTriangle, Check, ChevronDown, FolderOpen, Loader2, X } from "lucide-react";
import { InlineNotice } from "@/components/ui/inline-notice";
import { formatBytes } from "@/lib/format";
import { formatCooldownLabel, hostLockLabel } from "@/lib/downloadPresentation";
import { getVisibleProbeWarnings, simplifyUserMessage } from "@/lib/userFacingMessages";
import { cn } from "@/lib/utils";
import type { DownloadContentCategory, DownloadProbe } from "@/types/download";

const DOWNLOAD_CAPTURE_CATEGORIES: Array<{
	value: DownloadContentCategory;
	label: string;
}> = [
	{ value: "compressed", label: "Compressed" },
	{ value: "programs", label: "Programs" },
	{ value: "videos", label: "Videos" },
	{ value: "music", label: "Music" },
	{ value: "pictures", label: "Pictures" },
	{ value: "documents", label: "Documents" },
];

const PROBE_BYTE_FORMAT = { unknownLabel: "Unknown", integerAbove: 100 } as const;

export function DialogFormField({
	label,
	children,
	id,
}: {
	label: string;
	children: React.ReactNode;
	id: string;
}) {
	return (
		<div className="flex flex-col gap-1">
			<Label.Root htmlFor={id} className="text-[10.5px] font-medium text-muted-foreground/50 uppercase tracking-widest">
				{label}
			</Label.Root>
			{children}
		</div>
	);
}

export function DialogInput({
	id,
	...props
}: React.InputHTMLAttributes<HTMLInputElement> & { id: string }) {
	return (
		<input
			id={id}
			{...props}
			className={cn(
				"w-full rounded-md border border-border bg-[hsl(var(--card))] px-3 h-8 text-[12.5px] text-foreground placeholder:text-muted-foreground/40 outline-none",
				"focus:border-primary/60 focus:ring-1 focus:ring-primary/30 transition-colors",
				props.className,
			)}
		/>
	);
}

export function CompactFieldLabel({
	htmlFor,
	children,
}: {
	htmlFor?: string;
	children: React.ReactNode;
}) {
	return (
		<Label.Root
			htmlFor={htmlFor}
			className="text-[9.5px] font-semibold text-muted-foreground/40 uppercase tracking-[0.1em]"
		>
			{children}
		</Label.Root>
	);
}

export function CompactInput(
	props: React.InputHTMLAttributes<HTMLInputElement> & { id: string },
) {
	const { className, ...rest } = props;
	return (
		<input
			{...rest}
			className={cn(
				"h-[26px] w-full rounded-[3px] border border-border bg-[hsl(var(--card))] px-2 text-[11.5px] text-foreground",
				"placeholder:text-muted-foreground/30 outline-none",
				"focus:border-primary/50 focus:ring-1 focus:ring-primary/20 transition-colors",
				className,
			)}
		/>
	);
}

function DownloadCaptureCategorySelect({
	value,
	onChange,
	variant,
}: {
	value: DownloadContentCategory;
	onChange: (value: DownloadContentCategory) => void;
	variant: "dialog" | "compact";
}) {
	const compact = variant === "compact";

	return (
		<Select.Root value={value} onValueChange={(next) => onChange(next as DownloadContentCategory)}>
			<Select.Trigger
				className={cn(
					"flex items-center justify-between outline-none w-full transition-colors",
					compact
						? "h-[26px] rounded-[3px] border border-border bg-[hsl(var(--card))] px-2 text-[11.5px] text-foreground focus:border-primary/50"
						: "rounded-md border border-border bg-[hsl(var(--card))] px-3 h-8 text-[12.5px] text-foreground focus:border-primary/60 focus:ring-1 focus:ring-primary/30 data-[placeholder]:text-muted-foreground/40",
				)}
			>
				<Select.Value />
				<Select.Icon>
					<ChevronDown size={compact ? 10 : 13} className={compact ? "opacity-35" : "opacity-50"} />
				</Select.Icon>
			</Select.Trigger>
			<Select.Portal>
				<Select.Content
					position="popper"
					sideOffset={compact ? 3 : 4}
					side={compact ? "top" : undefined}
					avoidCollisions={compact}
					className={cn(
						compact
							? "z-[200] w-[var(--radix-select-trigger-width)] rounded-[3px] border border-border/80 py-0.5 bg-[hsl(var(--popover))] shadow-[0_8px_28px_rgba(0,0,0,0.55)]"
							: "z-50 w-[var(--radix-select-trigger-width)] rounded-md border border-border py-1 bg-[hsl(var(--card))] shadow-xl shadow-black/40 data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95",
					)}
				>
					<Select.Viewport>
						{DOWNLOAD_CAPTURE_CATEGORIES.map((cat) => (
							<Select.Item
								key={cat.value}
								value={cat.value}
								className={cn(
									compact
										? "flex cursor-default select-none items-center gap-2 px-2 py-1 text-[11.5px] text-foreground outline-none data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground"
										: "flex items-center justify-between px-3 py-[5px] text-[12px] cursor-default outline-none rounded-sm mx-0.5 text-foreground/80 data-[highlighted]:bg-accent data-[highlighted]:text-foreground",
								)}
							>
								<Select.ItemText>{cat.label}</Select.ItemText>
								<Select.ItemIndicator className={compact ? "ml-auto" : undefined}>
									<Check size={compact ? 10 : 12} className={compact ? undefined : "text-primary"} />
								</Select.ItemIndicator>
							</Select.Item>
						))}
					</Select.Viewport>
				</Select.Content>
			</Select.Portal>
		</Select.Root>
	);
}

function CapabilityBadge({ probe }: { probe: DownloadProbe }) {
	const label = probe.segmented
		? `${probe.plannedConnections}-Way Segmented`
		: probe.resumable
			? "Resume Ready"
			: probe.rangeSupported
				? "Single Session"
				: "Single Connection";

	return (
		<span
			className={cn(
				"inline-flex items-center rounded px-1.5 py-0.5 text-[9.5px] font-semibold uppercase tracking-wide",
				probe.segmented || probe.resumable
					? "bg-[hsl(var(--status-downloading)/0.14)] text-[hsl(var(--status-downloading))]"
					: probe.rangeSupported
						? "bg-[hsl(var(--status-paused)/0.14)] text-[hsl(var(--status-paused))]"
						: "bg-[hsl(var(--status-error)/0.12)] text-[hsl(var(--status-error)/0.80)]",
			)}
		>
			{label}
		</span>
	);
}

function MetaBadge({
	label,
	tone = "neutral",
}: {
	label: string;
	tone?: "neutral" | "good" | "warn";
}) {
	return (
		<span
			className={cn(
				"inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-medium tracking-wide",
				tone === "good" && "bg-[hsl(var(--status-downloading)/0.12)] text-[hsl(var(--status-downloading))]",
				tone === "warn" && "bg-[hsl(var(--status-paused)/0.14)] text-[hsl(var(--status-paused))]",
				tone === "neutral" && "bg-white/[0.08] text-foreground/65",
			)}
		>
			{label}
		</span>
	);
}

type ProbeSource = "live" | "cached" | "fallback" | null;

function detectProbeSource(warnings: string[]): ProbeSource {
	for (const warning of warnings) {
		if (warning.includes("Probe metadata source: live network probe.")) {
			return "live";
		}
		if (warning.includes("Probe metadata source: recent probe cache reuse.")) {
			return "cached";
		}
		if (warning.includes("Probe metadata source: planning fallback without fresh metadata.")) {
			return "fallback";
		}
	}
	return null;
}

function isGuardedProbeWarning(message: string): boolean {
	return message.toLowerCase().includes("guarded single-stream mode");
}

function conciseProbeWarning(message: string): string {
	if (isGuardedProbeWarning(message)) {
		return "VDM is keeping this transfer on one connection until resume support is proven for this exact request.";
	}
	return simplifyUserMessage(message);
}

function probeMetaBadges(probe: DownloadProbe): Array<{
	label: string;
	tone: "neutral" | "good" | "warn";
}> {
	const rows: Array<{ label: string; tone: "neutral" | "good" | "warn" }> = [];
	// Only show fallback probe source — live/cached are internal implementation details
	if (detectProbeSource(probe.warnings) === "fallback") {
		rows.push({ label: "Probe: fallback", tone: "warn" });
	}
	if (probe.compatibility.directUrlRecovered) {
		rows.push({ label: "Wrapper recovered", tone: "good" });
	} else if (probe.compatibility.browserInterstitialOnly) {
		rows.push({ label: "Browser interstitial", tone: "warn" });
	}
	// Only show free-space badge when space is actually insufficient
	if (probe.availableSpace !== null && probe.size !== null && probe.availableSpace < probe.size) {
		rows.push({
			label: `${formatBytes(probe.availableSpace, PROBE_BYTE_FORMAT)} free – may not fit`,
			tone: "warn",
		});
	}
	if (probe.hostDiagnostics.hardNoRange || !probe.rangeSupported) {
		rows.push({ label: "No-range host", tone: "warn" });
	}
	const cooldown = formatCooldownLabel(probe.hostDiagnostics.cooldownUntil);
	if (cooldown) {
		rows.push({ label: cooldown, tone: "warn" });
	}
	if (probe.hostDiagnostics.concurrencyLocked) {
		rows.push({ label: hostLockLabel(probe.hostDiagnostics.lockReason), tone: "warn" });
	}
	return rows.slice(0, 4);
}

export function ProbeSummaryStrip({
	loading,
	probe,
	error,
}: {
	loading: boolean;
	probe: DownloadProbe | null;
	error: string | null;
}) {
	if (!loading && !probe && !error) {
		return null;
	}

	const visibleWarnings = getVisibleProbeWarnings(probe?.warnings ?? [], 2).map(conciseProbeWarning);
	const metaBadges = probe ? probeMetaBadges(probe) : [];
	const simplifiedError = error ? simplifyUserMessage(error) : null;

	return (
		<div
			className={cn(
				"rounded-md px-3 py-2 transition-all",
				error
					? "bg-[hsl(var(--status-error)/0.07)] border border-[hsl(var(--status-error)/0.22)]"
					: "border border-border/40 bg-[hsl(0,0%,7%)]",
			)}
		>
			{loading ? (
				<div className="flex items-center gap-2 text-[11.5px] text-muted-foreground/45">
					<Loader2 size={11} className="animate-spin text-muted-foreground/35" />
					<span>Detecting…</span>
				</div>
			) : simplifiedError ? (
				<div className="flex items-center gap-1.5 text-[11.5px] text-[hsl(var(--status-error))]" title={error ?? undefined}>
					<AlertTriangle size={11} className="shrink-0" />
					<span>{simplifiedError}</span>
				</div>
			) : probe ? (
				<div className="flex flex-col gap-1.5">
					<div className="flex items-center justify-between gap-3">
						<div className="flex items-center gap-2 min-w-0">
							<CapabilityBadge probe={probe} />
							{probe.size !== null ? (
								<span className="text-[11px] text-foreground/55 tabular-nums shrink-0">
									{formatBytes(probe.size, PROBE_BYTE_FORMAT)}
								</span>
							) : null}
							{probe.host ? (
								<span className="text-[10.5px] text-foreground/55 max-w-[150px] truncate font-medium">
									{probe.host}
								</span>
							) : null}
						</div>
						<span
							className="shrink-0 rounded bg-white/[0.04] px-1.5 py-0.5 text-[10px] text-muted-foreground/38 truncate max-w-[140px]"
							title={probe.suggestedName}
						>
							{probe.suggestedName}
						</span>
					</div>
					{visibleWarnings.length > 0 ? (
						<div className="flex flex-col gap-1">
							{visibleWarnings.map((warning, index) => (
								<div
									key={`${warning}-${index}`}
									className="flex items-start gap-1.5 text-[10.5px] text-[hsl(var(--status-paused))]"
								>
									<AlertTriangle size={10} className="mt-[1px] shrink-0" />
									<span className="leading-snug">{warning}</span>
								</div>
							))}
						</div>
					) : null}
					{metaBadges.length > 0 ? (
						<div className="flex flex-wrap gap-1 pt-0.5">
							{metaBadges.map((badge) => (
								<MetaBadge key={badge.label} label={badge.label} tone={badge.tone} />
							))}
						</div>
					) : null}
				</div>
			) : null}
		</div>
	);
}

interface DuplicateActions {
	active: boolean;
	title?: string;
	detail?: string;
	primaryLabel: string;
	onPrimary: () => void;
	secondaryLabel?: string;
	onSecondary?: () => void;
}

interface DownloadCapturePaneProps {
	variant: "dialog" | "compact";
	category: DownloadContentCategory;
	onCategoryChange: (value: DownloadContentCategory) => void;
	savePath: string;
	onSavePathChange: (value: string) => void;
	onBrowseSavePath: () => void;
	filename: string;
	onFilenameChange: (value: string) => void;
	filenamePlaceholder: string;
	sizeLabel?: string | null;
	warningMessage?: string | null;
	errorMessage?: string | null;
	onWarningDismiss?: () => void;
	onErrorDismiss?: () => void;
	duplicateActions?: DuplicateActions;
	hideWarningWhenDuplicate?: boolean;
	filenameResetVisible?: boolean;
	onFilenameReset?: () => void;
	fieldIds?: {
		category: string;
		savePath: string;
		filename: string;
	};
}

export function DownloadCapturePane({
	variant,
	category,
	onCategoryChange,
	savePath,
	onSavePathChange,
	onBrowseSavePath,
	filename,
	onFilenameChange,
	filenamePlaceholder,
	sizeLabel,
	warningMessage,
	errorMessage,
	onWarningDismiss,
	onErrorDismiss,
	duplicateActions,
	hideWarningWhenDuplicate = false,
	filenameResetVisible = false,
	onFilenameReset,
	fieldIds,
}: DownloadCapturePaneProps) {
	const ids = fieldIds ?? {
		category: variant === "compact" ? "capture-category" : "download-category",
		savePath: variant === "compact" ? "capture-savepath" : "download-savepath",
		filename: variant === "compact" ? "capture-filename" : "download-filename",
	};
	const resolvedWarningMessage = warningMessage ? simplifyUserMessage(warningMessage) : null;
	const resolvedErrorMessage = errorMessage ? simplifyUserMessage(errorMessage) : null;

	if (variant === "dialog") {
		return (
			<>
				<DialogFormField label="Filename" id={ids.filename}>
					<div className="relative">
						<DialogInput
							id={ids.filename}
							type="text"
							placeholder={filenamePlaceholder}
							value={filename}
							onChange={(event) => onFilenameChange(event.target.value)}
							className={cn(filenameResetVisible && "pr-7")}
						/>
						{filenameResetVisible && onFilenameReset ? (
							<button
								type="button"
								title="Reset to auto-detected name"
								className="absolute right-1.5 top-1/2 -translate-y-1/2 flex h-5 w-5 items-center justify-center rounded text-muted-foreground/50 hover:text-foreground hover:bg-accent/60 transition-colors"
								onClick={onFilenameReset}
							>
								<X size={11} strokeWidth={2} />
							</button>
						) : null}
					</div>
				</DialogFormField>

				<div className="grid grid-cols-2 gap-3">
					<DialogFormField label="Category" id={ids.category}>
						<DownloadCaptureCategorySelect
							value={category}
							onChange={onCategoryChange}
							variant="dialog"
						/>
					</DialogFormField>

					<DialogFormField label="Save to" id={ids.savePath}>
						<div className="relative">
							<DialogInput
								id={ids.savePath}
								type="text"
								value={savePath}
								onChange={(event) => onSavePathChange(event.target.value)}
								placeholder="Choose a folder"
								className="pr-8"
							/>
							<button
								type="button"
								className="absolute right-1.5 top-1/2 -translate-y-1/2 flex h-5 w-5 items-center justify-center rounded text-muted-foreground/50 hover:text-foreground hover:bg-accent/60 transition-colors"
								onClick={onBrowseSavePath}
							>
								<FolderOpen size={12} strokeWidth={1.8} />
							</button>
						</div>
					</DialogFormField>
				</div>

				{resolvedErrorMessage ? (
					<InlineNotice
						tone="error"
						message={resolvedErrorMessage}
						onDismiss={onErrorDismiss}
						className="text-[11.5px]"
					/>
				) : null}
			</>
		);
	}

	const duplicateActive = duplicateActions?.active ?? false;
	const showWarning = Boolean(resolvedWarningMessage) && !(duplicateActive && hideWarningWhenDuplicate);

	return (
		<>
			<div className="flex items-end gap-2">
				<div className="flex-1 flex flex-col gap-0.5">
					<CompactFieldLabel htmlFor={ids.category}>Category</CompactFieldLabel>
					<DownloadCaptureCategorySelect
						value={category}
						onChange={onCategoryChange}
						variant="compact"
					/>
				</div>

				{sizeLabel ? (
					<div className="flex flex-col items-end gap-0.5 shrink-0">
						<CompactFieldLabel>Size</CompactFieldLabel>
						<span className="flex h-[26px] items-center text-[11.5px] text-foreground/72 font-mono tabular-nums">
							{sizeLabel}
						</span>
					</div>
				) : null}
			</div>

			<div className="flex flex-col gap-0.5">
				<CompactFieldLabel htmlFor={ids.savePath}>Save to</CompactFieldLabel>
				<div className="flex items-center gap-1">
					<CompactInput
						id={ids.savePath}
						value={savePath}
						onChange={(event) => onSavePathChange(event.target.value)}
						placeholder="Default download folder"
						className="flex-1"
					/>
					<button
						type="button"
						onClick={onBrowseSavePath}
						className="flex h-[26px] w-[26px] items-center justify-center rounded-[3px] border border-border bg-[hsl(var(--card))] text-muted-foreground/45 hover:bg-accent hover:text-foreground transition-colors shrink-0"
					>
						<FolderOpen size={12} />
					</button>
				</div>
			</div>

			<div className="flex flex-col gap-0.5">
				<CompactFieldLabel htmlFor={ids.filename}>File name</CompactFieldLabel>
				<div className="relative">
					<CompactInput
						id={ids.filename}
						value={filename}
						onChange={(event) => onFilenameChange(event.target.value)}
						placeholder={filenamePlaceholder}
						className={cn(filenameResetVisible && "pr-7")}
					/>
					{filenameResetVisible && onFilenameReset ? (
						<button
							type="button"
							title="Reset to auto-detected name"
							onClick={onFilenameReset}
							className="absolute right-1 top-1/2 flex h-[18px] w-[18px] -translate-y-1/2 items-center justify-center rounded-[3px] text-muted-foreground/34 transition-colors hover:bg-accent hover:text-foreground/75"
						>
							<X size={10} />
						</button>
					) : null}
				</div>
			</div>

			{showWarning ? (
				<InlineNotice
					tone="warning"
					message={resolvedWarningMessage}
					onDismiss={onWarningDismiss}
					className="rounded-[3px] px-2 py-1.5 text-[10.5px]"
				/>
			) : null}

			{duplicateActive ? (
				<div className="flex flex-col gap-1.5 rounded-[4px] border border-[hsl(var(--status-paused)/0.18)] bg-[hsl(var(--status-paused)/0.07)] px-2 py-1.5">
					<div className="flex items-start gap-1.5">
						<AlertTriangle size={9} className="mt-[2px] shrink-0 text-[hsl(var(--status-paused))]" />
						<div className="min-w-0 flex-1">
							<div className="text-[10.5px] font-medium leading-[1.25] text-foreground/84">
								{duplicateActions?.title ?? "This file already exists"}
							</div>
							{duplicateActions?.detail ? (
								<div className="mt-[1px] text-[9px] leading-[1.3] text-muted-foreground/46">
									{duplicateActions.detail}
								</div>
							) : null}
						</div>
					</div>
					<div className="flex flex-wrap gap-1">
						<button
							type="button"
							onClick={duplicateActions?.onPrimary}
							className="rounded-[4px] border border-[hsl(var(--primary)/0.28)] bg-[hsl(var(--primary)/0.08)] px-2 py-[3px] text-[10px] font-medium text-foreground/84 transition-colors hover:bg-[hsl(var(--primary)/0.14)]"
						>
							{duplicateActions?.primaryLabel}
						</button>
						{duplicateActions?.secondaryLabel && duplicateActions.onSecondary ? (
							<button
								type="button"
								onClick={duplicateActions.onSecondary}
								className="rounded-[4px] border border-border/70 px-2 py-[3px] text-[10px] text-muted-foreground/72 transition-colors hover:bg-accent hover:text-foreground"
							>
								{duplicateActions.secondaryLabel}
							</button>
						) : null}
					</div>
				</div>
			) : null}

			{resolvedErrorMessage && !duplicateActive ? (
				<InlineNotice
					tone="error"
					message={resolvedErrorMessage}
					onDismiss={onErrorDismiss}
					className="rounded-[3px] px-2 py-1.5 text-[10.5px]"
				/>
			) : null}
		</>
	);
}
