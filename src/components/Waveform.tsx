const BARS = 24;

/** Decorative live-audio wave. Phase 3 will drive `level` from real mic RMS. */
export default function Waveform({ level = 0.5 }: { level?: number }) {
  return (
    <div role="img" aria-label="Audio level" className="flex h-6 items-center gap-[3px]">
      {Array.from({ length: BARS }, (_, i) => (
        <span
          key={i}
          className="w-[3px] rounded-full bg-rose-400 animate-pulse"
          style={{
            height: `${20 + Math.abs(Math.sin(i * 0.9)) * 80 * level}%`,
            animationDelay: `${(i % 6) * 80}ms`,
          }}
        />
      ))}
    </div>
  );
}
