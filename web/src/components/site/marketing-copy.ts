import { useTranslation } from 'react-i18next'

export function useMarketingCopy() {
  const { t } = useTranslation()
  return t('marketing', { returnObjects: true })
}
