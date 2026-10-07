export function Logo({ className = 'size-7' }: { className?: string }) {
  // The shared Beans mark stays transparent on light and dark surfaces.
  return (
    <img src="/brand/beans-mark.svg" width={1254} height={1254} className={className} alt="" aria-hidden="true" />
  )
}
