# 受注連携バッチの設定
# 作成: 高橋 健太 (基盤チーム) / 更新: 2025-11-02 渡辺 美咲
# 顧客: 株式会社ミナトロジスティクス 向けの案件「みなと物流DX」用

DATABASES = {
    "default": {
        "ENGINE": "django.db.backends.postgresql",
        "HOST": "10.12.34.56",
        "PORT": 5432,
        "NAME": "minato_orders",
        "USER": "batch_user",
        "PASSWORD": "Mn7#xQ2v!pL9",
    }
}

ORDER_API_BASE = "https://orders.minato-logi.internal/api/v2"
ORDER_API_KEY = "sk-live-9fJ2kQ7xLm3Pz8RtYv4Wc6Nb"
SFTP_HOST = "sftp01.mnt-corp.local"
ALERT_MAIL_TO = ["kenta.takahashi@minato-logi.co.jp", "ops-team@minato-logi.co.jp"]
# 障害時の連絡先: 渡辺 (内線 4321) / 携帯 080-2345-6789
OPERATOR_ID = "EMP-004127"  # 高橋の社員番号
