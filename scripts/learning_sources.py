#!/usr/bin/env python3
"""Read existing source integrations and collect evidence; never modify provider state."""
import base64
import hashlib
import html.parser
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time
import urllib.parse

LIMIT = 32 * 1024 * 1024
# The caption and channel listing tool; the store may name its own (tests use a stand-in).
YTDLP = 'yt-dlp'
# The page fetcher for the public oEmbed fallback.
CURL = 'curl'

def read_json(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError('Source cache is missing or is not a regular file')
    with path.open('rb') as stream:
        data = stream.read(LIMIT + 1)
    if len(data) > LIMIT:
        raise ValueError('Source exceeds the 32 MiB bound')
    return json.loads(data)

def catalog(home):
    paths = [('news', home / '.local/state/omarchy-news/cache.json'),
             ('youtube', home / '.cache/omarchy/youtube-suggester/state.json')]
    providers, items, seen = [], [], set()
    for kind, path in paths:
        provider = dict(id=kind, name='News' if kind == 'news' else 'YouTube suggestions', available=False, count=0)
        try:
            data = read_json(path)
            provider.update(available=True, updated_at=data.get('updated_at'), modified_at=int(path.stat().st_mtime), source=str(path))
            rows = data.get('items', []) if kind == 'news' else data.get('recommendations', []) + data.get('pool', []) + data.get('recent', [])
            for row in rows:
                url = row.get('link' if kind == 'news' else 'url', '')
                if not url and kind == 'youtube' and re.fullmatch(r'[\w-]{11}', row.get('id', '')):
                    url = 'https://www.youtube.com/watch?v=' + row['id']
                if not valid_url(url) or url in seen:
                    continue
                seen.add(url)
                item = dict(id=hashlib.sha256(url.encode()).hexdigest()[:24], kind='article' if kind == 'news' else 'youtube-video', url=url,
                            title=str(row.get('title') or url), publisher=str(row.get('domain') or row.get('channel') or urllib.parse.urlsplit(url).hostname),
                            summary=str(row.get('summary') or row.get('description') or ''), duration=row.get('duration_formatted') or '',
                            published=row.get('age_label') or row.get('publishedMs'), provider=kind,
                            provider_saved=bool(row.get('bookmarked')), cached_at=provider['modified_at'], evidence='discovery_only',
                            signal='discovery' if kind == 'youtube' else 'news')
                items.append(item)
                provider['count'] += 1
        except (OSError, ValueError, TypeError) as error:
            provider['error'] = str(error)
        providers.append(provider)
    return dict(providers=providers, items=items[:500], notice='Existing integrations supply discovery. Full article text or video captions are collected separately before analysis.')

def valid_url(url):
    try:
        p = urllib.parse.urlsplit(url)
        return p.scheme in ('http', 'https') and bool(p.hostname) and not p.username and not p.password
    except ValueError:
        return False

class Article(html.parser.HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.skip = 0
        self.in_title = False
        self.title = []
        self.blocks = []
        self.current = []
        self.main_depth = 0
        self.main_blocks = []
    def flush(self):
        value = ' '.join(' '.join(self.current).split())
        self.current = []
        if value:
            self.blocks.append(value)
            if self.main_depth:
                self.main_blocks.append(value)
    def handle_starttag(self, tag, attrs):
        if tag in ('script', 'style', 'nav', 'header', 'footer', 'aside', 'noscript', 'svg'):
            self.skip += 1
        if tag == 'title': self.in_title = True
        if tag in ('article', 'main'):
            self.flush()
            self.main_depth += 1
        if tag in ('p', 'h1', 'h2', 'h3', 'li', 'blockquote', 'br', 'div'): self.flush()
    def handle_endtag(self, tag):
        if tag in ('p', 'h1', 'h2', 'h3', 'li', 'blockquote', 'div', 'article', 'main'): self.flush()
        if tag in ('article', 'main'): self.main_depth = max(0, self.main_depth - 1)
        if tag == 'title': self.in_title = False
        if tag in ('script', 'style', 'nav', 'header', 'footer', 'aside', 'noscript', 'svg'):
            self.skip = max(0, self.skip - 1)
    def handle_data(self, text):
        if self.in_title: self.title.append(text)
        elif not self.skip: self.current.append(text)

def fetch_article(url):
    if not valid_url(url): raise ValueError('Choose an HTTP or HTTPS article URL')
    with tempfile.TemporaryDirectory(prefix='soliton-article-') as directory:
        target = Path(directory) / 'source'
        response = subprocess.run(['curl', '--fail', '--silent', '--show-error', '--location', '--proto', '=https,http', '--proto-redir', '=https,http',
                                   '--max-time', '30', '--max-filesize', str(LIMIT), '--output', str(target), '--write-out', '%{content_type}', '--', url],
                                  capture_output=True, timeout=35)
        if response.returncode: raise ValueError('Article could not be fetched: ' + response.stderr.decode(errors='replace')[-500:])
        raw = target.read_bytes()
    if len(raw) > LIMIT: raise ValueError('Article exceeds the size bound')
    content_type = response.stdout.decode(errors='replace').lower()
    if 'html' not in content_type and 'text/plain' not in content_type:
        raise ValueError('This source is not HTML or plain text; import its document explicitly')
    body = raw.decode('utf-8', errors='replace')
    if 'html' in content_type:
        parser = Article()
        parser.feed(body)
        parser.flush()
        blocks = parser.main_blocks or parser.blocks
        title = ' '.join(parser.title).strip()
        method = 'article/main HTML text' if parser.main_blocks else 'page text; no article/main landmark'
    else:
        blocks = [p.strip() for p in body.split('\n\n') if p.strip()]
        title, method = url, 'plain text'
    # Keep paragraph-sized citations; long blocks are losslessly divided.
    segments = []
    for block in blocks:
        for offset in range(0, len(block), 6000):
            part = block[offset:offset + 6000]
            segments.append(dict(id=f'p{len(segments)+1}', text=part, index=len(segments), url=url))
    if sum(len(s['text']) for s in segments) < 120:
        raise ValueError('Too little readable article text was returned; the source may require a login or JavaScript')
    if len(segments) > 20000: raise ValueError('Article exceeds the segment bound')
    return dict(title=title or url, segments=segments, coverage=dict(state='complete', extraction=method, notice='Fetched page text; extraction is not independent verification.'), raw_base64=base64.b64encode(raw).decode())

def youtube_id(url):
    if not valid_url(url): raise ValueError('Choose a YouTube video URL')
    p = urllib.parse.urlsplit(url)
    host = (p.hostname or '').lower()
    value = p.path.strip('/').split('/')[0] if host == 'youtu.be' else urllib.parse.parse_qs(p.query).get('v', [''])[0]
    if host not in ('youtu.be','youtube.com','www.youtube.com','m.youtube.com') or not re.fullmatch(r'[\w-]{11}', value):
        raise ValueError('Choose a single YouTube video URL')
    return value

def fetch_video(url):
    video_id = youtube_id(url)
    with tempfile.TemporaryDirectory(prefix='soliton-captions-') as directory:
        target = Path(directory)
        process = subprocess.run([YTDLP, '--ignore-config', '--no-playlist', '--skip-download', '--write-info-json', '--write-subs', '--write-auto-subs',
                                  '--sub-langs', 'en.*,en,-live_chat', '--sub-format', 'json3', '--socket-timeout', '20', '--retries', '1', '--max-filesize', '32M',
                                  '-o', str(target / '%(id)s.%(ext)s'), '--', 'https://www.youtube.com/watch?v=' + video_id],
                                 stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=150)
        infos = list(target.glob('*.info.json'))
        captions = sorted(target.glob('*.json3'))
        if not infos or not captions:
            raise ValueError('No accessible English captions were returned. ' + process.stderr.decode(errors='replace')[-500:])
        info = read_json(infos[0])
        selected = captions[0]
        data = read_json(selected)
        raw = selected.read_bytes()
        segments = []
        for event in data.get('events', []):
            text = ''.join(str(s.get('utf8', '')) for s in event.get('segs', [])).strip()
            if not text: continue
            start = int(event.get('tStartMs', 0)); end = start + int(event.get('dDurationMs', 0))
            segments.append(dict(id=f't{start}-{len(segments)}', index=len(segments), text=text, start_ms=start, end_ms=end, url=url))
        if not segments: raise ValueError('The returned caption track contains no text')
        if len(segments) > 20000: raise ValueError('Caption track exceeds 20,000 segments; split the source for collection')
        return dict(title=info.get('title') or url, segments=segments, channel=info.get('channel'), duration_seconds=info.get('duration'),
                    coverage=dict(state='complete', caption_track=selected.name, notice='All returned caption cues are retained. A complete caption track does not establish that every spoken word was captioned.'),
                    raw_base64=base64.b64encode(raw).decode())

def youtube_meta(url):
    """A video's own title, channel, category, tags and description, read without
    downloading it; the public oEmbed page names a video the downloader cannot open."""
    watch = 'https://www.youtube.com/watch?v=' + youtube_id(url)
    process = subprocess.run([YTDLP, '--ignore-config', '--no-playlist', '--no-cookies', '--no-cookies-from-browser', '--skip-download',
                              '--socket-timeout', '20', '--retries', '1', '-J', '--', watch],
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60)
    if process.returncode == 0 and len(process.stdout) <= LIMIT:
        try:
            info = json.loads(process.stdout)
            return dict(via='yt-dlp', id=info.get('id'), title=str(info.get('title') or ''), channel=str(info.get('channel') or info.get('uploader') or ''),
                        categories=[str(c) for c in (info.get('categories') or [])][:8], tags=[str(t) for t in (info.get('tags') or [])][:64],
                        description=str(info.get('description') or '')[:1500], duration=info.get('duration'))
        except ValueError:
            pass
    error = process.stderr.decode(errors='replace').strip()[-300:]
    response = subprocess.run([CURL, '--fail', '--silent', '--show-error', '--location', '--proto', '=https', '--proto-redir', '=https', '--max-time', '20',
                               '--max-filesize', '65536', '--', 'https://www.youtube.com/oembed?format=json&url=' + urllib.parse.quote(watch, safe='')],
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=25)
    if response.returncode == 0:
        try:
            data = json.loads(response.stdout)
            return dict(via='oEmbed', id=watch[-11:], title=str(data.get('title') or ''), channel=str(data.get('author_name') or ''),
                        categories=[], tags=[], description='', downloader_error=error)
        except ValueError:
            pass
    raise ValueError('The video\'s metadata could not be read: ' + (error or response.stderr.decode(errors='replace').strip()[-300:]))

def fetch_channel(url, learn=False, known=()):
    if not valid_url(url): raise ValueError('Choose a public YouTube channel URL')
    parsed = urllib.parse.urlsplit(url)
    if parsed.hostname not in ('youtube.com', 'www.youtube.com', 'm.youtube.com') or not re.match(r'^/(?:@[^/]+|channel/UC[\w-]+)(?:/videos)?/?$', parsed.path):
        raise ValueError('Use a YouTube @handle or /channel/ URL')
    response = subprocess.run([YTDLP, '--ignore-config', '--flat-playlist', '--playlist-end', '20', '--dump-single-json', '--skip-download', '--socket-timeout', '20', '--retries', '1', '--', url.rstrip('/') + ('' if parsed.path.rstrip('/').endswith('/videos') else '/videos')],
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=150)
    if response.returncode: raise ValueError('Channel discovery failed: ' + response.stderr.decode(errors='replace')[-500:])
    if len(response.stdout)>LIMIT: raise ValueError('Channel response exceeds the size bound')
    data=json.loads(response.stdout)
    children=[]
    segments=[]
    deadline=time.monotonic()+480
    channel=data.get('channel') or data.get('uploader') or ''
    for item in (data.get('entries') or [])[:20]:
        identity=item.get('id','')
        if not re.fullmatch(r'[\w-]{11}',identity):continue
        video='https://www.youtube.com/watch?v='+identity
        # Title and channel let the store's local rules judge each child before it is admitted.
        child = {'type':'youtube-video','url':video,'title':str(item.get('title') or ''),'channel':str(item.get('channel') or channel)}
        # With his category rules set, each child's own metadata is read here so
        # the store judges it without waiting; past the deadline it stays unknown.
        if learn and identity not in known and time.monotonic() < deadline:
            try: child['meta'] = youtube_meta(video)
            except Exception as error: child['meta'] = {'error': str(error)[:500]}
        children.append(child)
        segments.append({'id':identity,'text':item.get('title') or identity,'url':video,'index':len(segments)})
    if not children:raise ValueError('This channel returned no public video entries')
    return dict(title=data.get('title') or url, segments=segments, child_sources=children, channel=channel or None,
                coverage=dict(state='partial', notice='Discovery of the latest 20 public channel entries. Each caption track has its own durable job and coverage; these titles are not transcript evidence.'),
                raw_base64=base64.b64encode(response.stdout).decode())

def main():
    global YTDLP, CURL
    args = json.load(sys.stdin)
    if args.get('ytdlp'): YTDLP = str(args['ytdlp'])
    if args.get('curl'): CURL = str(args['curl'])
    action = args.get('action')
    if action == 'catalog': result = catalog(Path.home())
    elif action == 'article': result = fetch_article(args['url'])
    elif action == 'youtube-video': result = fetch_video(args['url'])
    elif action == 'youtube-channel': result = fetch_channel(args['url'], bool(args.get('learn')), set(args.get('known') or []))
    elif action == 'youtube-meta': result = youtube_meta(args['url'])
    else: raise ValueError('Unsupported learning source action')
    json.dump(result, sys.stdout, ensure_ascii=False)

if __name__ == '__main__':
    try: main()
    except Exception as error:
        json.dump({'error': str(error)}, sys.stdout)
        sys.exit(1)
