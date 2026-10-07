'use client'

import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import { Pause, Play } from 'lucide-react'
import { useTranslation } from 'react-i18next'

function subscribeMotion(change: () => void) {
  const query = matchMedia('(prefers-reduced-motion: reduce)')
  query.addEventListener('change', change)
  return () => query.removeEventListener('change', change)
}

const MotionContext = createContext({ paused: true, reduced: true })

export function useSiteMotion() {
  return useContext(MotionContext)
}

export function MotionExperience({ children }: { children: React.ReactNode }) {
  const root = useRef<HTMLDivElement>(null)
  const [paused, setPaused] = useState(false)
  const reduced = useSyncExternalStore(
    subscribeMotion,
    () => matchMedia('(prefers-reduced-motion: reduce)').matches,
    () => true,
  )
  const { t, i18n } = useTranslation()

  useEffect(() => {
    const element = root.current
    if (!element) return
    const observer = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (entry.target instanceof HTMLElement)
          entry.target.dataset.active = String(entry.isIntersecting)
      }
    })
    element
      .querySelectorAll('.motion-island')
      .forEach((island) => observer.observe(island))
    const visibility = () => {
      element.dataset.pageVisible = String(!document.hidden)
    }
    visibility()
    document.addEventListener('visibilitychange', visibility)
    return () => {
      observer.disconnect()
      document.removeEventListener('visibilitychange', visibility)
    }
  }, [])

  useEffect(() => {
    const element = root.current
    if (!element || reduced || paused || !element.querySelector('.beans-hero'))
      return
    let disposed = false
    let revert = () => {}
    void Promise.all([import('gsap'), import('gsap/ScrollTrigger')])
      .then(([{ gsap }, { ScrollTrigger }]) => {
        if (disposed) return
        gsap.registerPlugin(ScrollTrigger)
        const context = gsap.context(() => {
          const media = gsap.matchMedia()
          media.add('(prefers-reduced-motion: no-preference)', () => {
            gsap.from('.hero-copy > *', {
              y: 24,
              opacity: 0.55,
              duration: 0.85,
              stagger: 0.09,
              ease: 'power3.out',
              clearProps: 'all',
            })
            gsap.from('.hero-scene', {
              scale: 0.7,
              rotate: -18,
              duration: 1.3,
              ease: 'power3.out',
            })
            gsap.to('.reading-progress', {
              scaleX: 1,
              ease: 'none',
              scrollTrigger: {
                trigger: element,
                start: 'top top',
                end: 'bottom bottom',
                scrub: true,
              },
            })
            gsap.to('.hero-scene', {
              yPercent: 18,
              rotation: 8,
              ease: 'none',
              scrollTrigger: {
                trigger: '.beans-hero',
                start: 'top top',
                end: 'bottom top',
                scrub: 0.8,
              },
            })
            ScrollTrigger.create({
              trigger: '#relay',
              start: 'top bottom',
              end: 'bottom top',
              toggleClass: 'in-view',
            })
            gsap.fromTo(
              '.relay-orbit',
              { rotate: -28, scale: 0.85 },
              {
                rotate: 20,
                scale: 1.1,
                ease: 'none',
                scrollTrigger: {
                  trigger: '#relay',
                  start: 'top bottom',
                  end: 'bottom top',
                  scrub: 1,
                },
              },
            )
            gsap.fromTo(
              '.relay-node',
              { y: 100, rotateX: 22, scale: 0.88 },
              {
                y: 0,
                rotateX: 0,
                scale: 1,
                stagger: 0.12,
                ease: 'none',
                scrollTrigger: {
                  trigger: '.relay-diagram',
                  start: 'top 95%',
                  end: 'center 65%',
                  scrub: 0.8,
                },
              },
            )
            gsap.fromTo(
              '.relay-diagram .wire',
              { scale: 0.2 },
              {
                scale: 1,
                ease: 'none',
                scrollTrigger: {
                  trigger: '.relay-diagram',
                  start: 'top 90%',
                  end: 'center 60%',
                  scrub: 0.8,
                },
              },
            )
            for (const section of gsap.utils.toArray<HTMLElement>(
              '.story-heading',
            )) {
              gsap.from(section, {
                y: 35,
                duration: 0.85,
                ease: 'power3.out',
                scrollTrigger: {
                  trigger: section,
                  start: 'top 90%',
                  once: true,
                },
                clearProps: 'all',
              })
            }
            for (const [index, card] of gsap.utils
              .toArray<HTMLElement>('.bento-card')
              .entries()) {
              gsap.fromTo(
                card,
                { y: 65 + (index % 2) * 35, rotateX: 7, scale: 0.96 },
                {
                  y: 0,
                  rotateX: 0,
                  scale: 1,
                  ease: 'none',
                  scrollTrigger: {
                    trigger: card,
                    start: 'top 98%',
                    end: 'top 66%',
                    scrub: 0.6,
                  },
                },
              )
            }
            gsap.fromTo(
              '.example-workbench',
              { rotateX: 10, y: 55 },
              {
                rotateX: 0,
                y: 0,
                ease: 'none',
                scrollTrigger: {
                  trigger: '.story-examples',
                  start: 'top 90%',
                  end: 'center 65%',
                  scrub: 0.6,
                },
              },
            )
            gsap.from('.setup-step', {
              y: 28,
              stagger: 0.08,
              duration: 0.7,
              ease: 'power3.out',
              scrollTrigger: {
                trigger: '#start',
                start: 'top 80%',
                once: true,
              },
              clearProps: 'all',
            })
            for (const step of gsap.utils.toArray<HTMLElement>('.setup-step')) {
              ScrollTrigger.create({
                trigger: step,
                start: 'top 72%',
                end: 'bottom 25%',
                toggleClass: 'step-active',
              })
            }
            gsap.fromTo(
              '.cta-mark',
              { y: 60, rotate: -16, scale: 0.75 },
              {
                y: 0,
                rotate: 0,
                scale: 1,
                ease: 'none',
                scrollTrigger: {
                  trigger: '.beans-cta',
                  start: 'top bottom',
                  end: 'center 65%',
                  scrub: 0.8,
                },
              },
            )
          })
        }, element)
        revert = () => context.revert()
        ScrollTrigger.refresh()
      })
      .catch(() => {
        /* The fully rendered page remains usable without the enhancement. */
      })
    return () => {
      disposed = true
      revert()
    }
  }, [paused, reduced])

  return (
    <MotionContext.Provider value={{ paused, reduced }}>
      <div
        ref={root}
        className="beans-experience"
        lang={i18n.language}
        data-motion={reduced || paused ? 'still' : 'running'}
      >
        <div className="reading-progress" aria-hidden="true" />
        {children}
        {!reduced && (
          <button
            className="motion-control"
            type="button"
            onClick={() => setPaused(!paused)}
            aria-pressed={paused}
          >
            {paused ? (
              <Play size={14} aria-hidden="true" />
            ) : (
              <Pause size={14} aria-hidden="true" />
            )}
            <span>{t(paused ? 'motion.resume' : 'motion.pause')}</span>
          </button>
        )}
      </div>
    </MotionContext.Provider>
  )
}
