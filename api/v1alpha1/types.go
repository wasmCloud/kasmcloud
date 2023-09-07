package v1alpha1

import metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

type Claims struct {
	Issuer    string       `json:"issuer"`
	Subject   string       `json:"subject"`
	NotBefore *metav1.Time `json:"notBefore,omitempty"`
	IssuedAt  metav1.Time  `json:"issuedAt"`
	Expires   *metav1.Time `json:"expires"`
}
