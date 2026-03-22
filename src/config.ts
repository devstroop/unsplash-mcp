export interface UnsplashConfig {
  userAgent: string;
  requestDelay: number;
  cacheTtl: number;
  maxRetries: number;
  baseUrl: string;
}

const userAgents = [
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36',
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36',
  'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36',
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:121.0) Gecko/20100101 Firefox/121.0',
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:121.0) Gecko/20100101 Firefox/121.0',
];

export const config: UnsplashConfig = {
  userAgent: userAgents[Math.floor(Math.random() * userAgents.length)],
  requestDelay: 1000,
  cacheTtl: 300,
  maxRetries: 3,
  baseUrl: 'https://unsplash.com',
};

export function validateConfig(): void {}
