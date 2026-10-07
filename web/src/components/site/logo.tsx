export function Logo({ className = 'size-7' }: { className?: string }) {
  // The shared Beans mark stays transparent on light and dark surfaces.
  return (
    <img src="/brand/beans-mark.png" width={1280} height={1280} className={className} alt="" aria-hidden="true" />
  )
}
