-- 月次売上レポート（経理部 佐々木 由美 さん依頼、チケット FIN-2291）
-- 対象: 取引先コード C-00872（有限会社さくら製作所）、C-01544（東邦精機株式会社）
SELECT o.order_no,
       c.customer_name,
       e.employee_no
FROM   orders o
JOIN   customers c ON c.code = o.customer_code
JOIN   employees e ON e.id = o.sales_rep
WHERE  c.code IN ('C-00872', 'C-01544')
  AND  e.employee_no = 'S10458'   -- 営業担当 中村 翔
  AND  o.ordered_at >= '2026-09-01';
-- 接続先: rpt-db02.hq.sakura-net.lan (192.168.40.12) / 管理者 admin@sakura-net.lan
