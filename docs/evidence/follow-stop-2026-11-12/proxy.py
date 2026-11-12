import http.server,json,urllib.parse,urllib.request
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor
import threading
TARGET=15531675
ROOT=Path('/private/tmp/celestia-follow-stop-regression')
lock=threading.Lock()
class Proxy(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  try:
   with urllib.request.urlopen('https://rpc.celestia.pops.one'+self.path,timeout=20) as f:data=f.read()
   parsed=urllib.parse.urlparse(self.path)
   height=urllib.parse.parse_qs(parsed.query).get('height',[''])[0]
   changed=parsed.path=='/block' and height==str(TARGET)
   if changed:
    obj=json.loads(data);before=obj['result']['block']['header']['app_hash'];after=('0' if before[0]!='0' else '1')+before[1:]
    obj['result']['block']['header']['app_hash']=after;data=json.dumps(obj).encode()
    with lock:
     with (ROOT/'faults.jsonl').open('a') as output:output.write(json.dumps({'height':TARGET,'field':'app_hash','original':before,'injected':after})+'\n')
   self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
  except Exception as e:
   self.send_error(502,str(e))
 def log_message(self,*args):pass
http.server.ThreadingHTTPServer(('127.0.0.1',38657),Proxy).serve_forever()
