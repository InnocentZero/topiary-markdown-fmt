import { createInterface } from 'node:readline';
import { format, version } from 'prettier';

const baseline = {
  parser: 'markdown',
  proseWrap: 'preserve',
  embeddedLanguageFormatting: 'off',
};

const profiles = {
  default: { parser: 'markdown' },
  lf: baseline,
  auto: { ...baseline, endOfLine: 'auto' },
  indent4: { ...baseline, endOfLine: 'auto', tabWidth: 4 },
  tabs: { ...baseline, endOfLine: 'auto', useTabs: true },
  wrap: { ...baseline, endOfLine: 'auto', proseWrap: 'always' },
  unwrap: { ...baseline, endOfLine: 'auto', proseWrap: 'never' },
};

async function compare(source, names) {
  const results = {};
  for (const name of names) {
    try {
      if (!Object.hasOwn(profiles, name)) throw Error(`Unknown profile: ${name}`);
      results[name] = { output: await format(source, profiles[name]) };
    } catch (err) {
      results[name] = { error: String(err) };
    }
  }
  return { results };
}

if (process.argv.includes('--version')) {
  console.log(version);
} else {
  const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
  for await (const line of lines) {
    try {
      const { source, names } = JSON.parse(line);
      const result = process.argv.includes('--proximity')
        ? await compare(source, names)
        : { output: await format(source, baseline) };
      process.stdout.write(`${JSON.stringify(result)}\n`);
    } catch (err) {
      process.stdout.write(`${JSON.stringify({ error: String(err) })}\n`);
    }
  }
}
