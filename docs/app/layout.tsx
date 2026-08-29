import type { Metadata } from 'next'
import { Footer, Layout, Navbar } from 'nextra-theme-docs'
import { Head } from 'nextra/components'
import { getPageMap } from 'nextra/page-map'
import 'nextra-theme-docs/style.css'
import './styles.css'

export const metadata: Metadata = {
  title: {
    default: '大黄狗文档',
    template: '%s · 大黄狗文档'
  },
  description: '大黄狗 Windows 系统健康助手的使用与开发文档'
}

const navbar = <Navbar logo={<span className="brand"><span className="brand-mark">黄</span>大黄狗</span>} />
const footer = <Footer>大黄狗 · 本地优先的 Windows 系统健康助手</Footer>

export default async function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="zh-CN" dir="ltr" suppressHydrationWarning>
      <Head>
        <meta name="theme-color" content="#ca8a04" />
      </Head>
      <body>
        <Layout
          navbar={navbar}
          pageMap={await getPageMap()}
          docsRepositoryBase="https://github.com/fengjutian/DahuangDog/tree/main/docs"
          footer={footer}
          editLink="在 GitHub 上编辑此页"
          feedback={{ content: '有问题？提交反馈' }}
          sidebar={{ defaultMenuCollapseLevel: 1 }}
          toc={{ title: '本页内容' }}
        >
          {children}
        </Layout>
      </body>
    </html>
  )
}
