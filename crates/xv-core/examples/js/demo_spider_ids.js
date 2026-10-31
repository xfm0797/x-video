// 验证用 JS Spider：用 TVBox 的 csp 约定，通过宿主 req 直接访问真实 CMS 接口
// 作用：端到端验证沙箱（req / JSON / 返回结构映射），也可作为写 JS 源的模板

var SITE = 'https://cj.lziapi.com/api.php/provide/vod';

function api(params) {
  var url = SITE + params;
  var text = req(url, { headers: { 'User-Agent': PC_UA, 'Accept': 'application/json' } });
  try {
    return JSON.parse(text);
  } catch (e) {
    log('解析失败: ' + url + ' -> ' + text.slice(0, 80));
    return { list: [] };
  }
}

function homeContent(filter) {
  var j = api('?ac=list');
  var cls = [];
  var arr = j.class || [];
  for (var i = 0; i < arr.length; i++) {
    cls.push({ type_id: String(arr[i].type_id), type_name: String(arr[i].type_name) });
  }
  return { class: cls, list: [] };
}

function categoryContent(tid, pg, filter, extend) {
  var j = api('?ac=videolist&t=' + encodeURIComponent(tid) + '&pg=' + pg);
  var out = [];
  var arr = j.list || [];
  for (var i = 0; i < arr.length; i++) {
    var v = arr[i];
    out.push({
      vod_id: String(v.vod_id),
      vod_name: v.vod_name,
      vod_pic: v.vod_pic,
      vod_remarks: v.vod_remarks || ''
    });
  }
  return {
    list: out,
    page: pg,
    pagecount: j.pagecount || 1,
    total: j.total || out.length
  };
}

function detailContent(ids) {
  var j = api('?ac=videolist&ids=' + encodeURIComponent(ids[0]));
  var v = (j.list || [])[0];
  if (!v) return { list: [] };
  return {
    list: [{
      vod_id: String(v.vod_id),
      vod_name: v.vod_name,
      vod_pic: v.vod_pic,
      vod_year: v.vod_year,
      vod_area: v.vod_area,
      vod_actor: v.vod_actor,
      vod_director: v.vod_director,
      vod_content: v.vod_content,
      vod_play_from: v.vod_play_from,
      vod_play_from: 'ids',
      vod_play_url: (v.vod_play_url || '').split('#').map(function (ep) {
        var p = ep.split('$');
        var id = (p[1] || '').split('/share/').pop();
        return p[0] + '$' + id;
      }).join('$$$')
    }]
  };
}

function searchContent(key, quick) {
  var j = api('?ac=videolist&wd=' + encodeURIComponent(key));
  var out = [];
  var arr = j.list || [];
  for (var i = 0; i < arr.length; i++) {
    out.push({
      vod_id: String(arr[i].vod_id),
      vod_name: arr[i].vod_name,
      vod_pic: arr[i].vod_pic,
      vod_remarks: arr[i].vod_remarks || ''
    });
  }
  return { list: out };
}

function playerContent(flag, id, vipFlags) {
  if (id.indexOf('http') === 0) {
    return { parse: 0, url: id, header: {} };
  }
  return { parse: 1, url: 'https://v.lzcdn27.com/share/' + id, header: {} };
}
