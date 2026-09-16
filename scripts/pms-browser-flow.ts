import { chromium } from 'playwright';

const PMS_URL = 'http://pms.powerpms.net:1801/sysin.html';
const PASSWORD = 'Admin@1234';

async function run() {
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();

  console.log('=== Step 1: Open PMS login ===');
  await page.goto(PMS_URL, { waitUntil: 'networkidle', timeout: 30000 });
  console.log('Page title:', await page.title());
  await page.screenshot({ path: '.tmp-pms-step1-login.png' });

  console.log('\n=== Step 2: Login as SJ ===');
  const userInput = page.locator('input[type="text"], input[name*="user"], input[placeholder*="用户"]').first();
  const passInput = page.locator('input[type="password"]').first();
  
  if (await userInput.count() > 0) {
    await userInput.fill('SJ');
    await passInput.fill(PASSWORD);
    const loginBtn = page.locator('button:has-text("登录"), button:has-text("Login"), input[type="submit"]').first();
    if (await loginBtn.count() > 0) {
      await loginBtn.click();
      await page.waitForTimeout(3000);
    }
  } else {
    console.log('Login form not found, trying iframe...');
    const frames = page.frames();
    console.log(`Found ${frames.length} frames`);
    for (const frame of frames) {
      const fi = frame.locator('input[type="text"]').first();
      if (await fi.count() > 0) {
        console.log('Found input in frame:', frame.url());
        await fi.fill('SJ');
        const fp = frame.locator('input[type="password"]').first();
        if (await fp.count() > 0) await fp.fill(PASSWORD);
        const fb = frame.locator('button').first();
        if (await fb.count() > 0) await fb.click();
        await page.waitForTimeout(3000);
        break;
      }
    }
  }

  console.log('After login title:', await page.title());
  console.log('URL:', page.url());
  await page.screenshot({ path: '.tmp-pms-step2-loggedin.png' });

  console.log('\n=== Step 3: Navigate to review list ===');
  const reviewLinks = await page.locator('a, span, div').filter({ hasText: /校审|审核|review|三维/i }).all();
  console.log(`Found ${reviewLinks.length} potential review navigation elements`);
  for (const link of reviewLinks.slice(0, 5)) {
    const text = await link.textContent();
    console.log(`  - "${text?.trim().substring(0, 50)}"`);
  }
  
  if (reviewLinks.length > 0) {
    await reviewLinks[0].click();
    await page.waitForTimeout(2000);
    await page.screenshot({ path: '.tmp-pms-step3-review.png' });
  }

  console.log('\n=== Page HTML snippet ===');
  const bodyText = await page.locator('body').textContent();
  console.log(bodyText?.substring(0, 500));

  await browser.close();
  console.log('\n=== Done ===');
}

run().catch(console.error);
