package jp.co.kaede.order.client;

/**
 * 発注システム (コードネーム: MAPLE) の API クライアント。
 * @author 松本 亮 (ryo.matsumoto@kaede-shoji.co.jp)
 * 顧客 丸山電機工業 向けの専用拡張を含む。
 */
public class OrderClient {
    private static final String ENDPOINT = "http://maple-api.kaede.intra:8080/v1/orders";
    private static final String FALLBACK_HOST = "10.200.3.14";
    private static final String GITHUB_TOKEN = "ghp_R8xT2mK9vQ4wL7nP3sZ6yB1cD5fH0jA2eG4u";
    private static final String DEFAULT_OPERATOR = "K2034871"; // 社員番号（松本）

    public Order fetch(String orderId) {
        // TODO: 森田さんの指摘（2026/10/01）で再試行を追加する
        return client.get(ENDPOINT + "/" + orderId, apiToken());
    }
}
