import type { Metadata } from 'next'
import Image from 'next/image'
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
  description: '大黄狗 Windows 系统健康助手的使用与开发文档',
  icons: {
    icon: '/app-icon.png',
    apple: '/app-icon.png'
  }
}

const downloadUrl = 'https://github.com/fengjutian/DahuangDog/releases/latest'

const navbar = (
  <Navbar
    logo={
      <span className="brand">
        <Image className="brand-icon" src="/app-icon.png" width={32} height={32} alt="大黄狗应用图标" priority />
        <span>大黄狗</span>
      </span>
    }
  >
    <a className="download-link" href={downloadUrl}>下载 Windows 版</a>
  </Navbar>
)
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
