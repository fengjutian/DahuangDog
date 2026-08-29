import nextra from 'nextra'

const withNextra = nextra({})

export default withNextra({
  output: 'export',
  outputFileTracingRoot: process.cwd(),
  images: {
    unoptimized: true
  }
})
